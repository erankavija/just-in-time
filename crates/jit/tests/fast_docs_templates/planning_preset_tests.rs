//! Integration tests for a project's own gate presets.
//!
//! Every preset is declared by the project under `.jit/config/gate-presets/`:
//! these cases exercise capturing one, listing it, showing it, and applying it,
//! and the audit events an application that writes the gate registry appends.

use jit::commands::test_helpers::ExecutorTestAccess;
use jit::declarations::{GateMode, GateStage};
use jit::domain::Priority;
use jit::storage::{IssueStore, JsonFileStorage};
use jit::test_utils::{declaration_dir, file_executor};
use jit::CommandExecutor;
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn test_preset_readers_agree_across_backends_and_apply_the_shown_gates() {
    use jit::gate_presets::{GatePresetDefinition, GateTemplate, PresetInfo};
    use jit::repository_state::VirtualPath;
    use jit::storage::RepositoryStateStore;
    use jit::test_harness::TestHarness;

    fn read_and_apply<S: IssueStore + RepositoryStateStore>(
        executor: &CommandExecutor<S>,
    ) -> (Vec<PresetInfo>, Vec<GatePresetDefinition>) {
        let listed = executor.gates().list_gate_presets().unwrap();
        let shown = listed
            .iter()
            .map(|info| {
                let preset = executor.gates().show_gate_preset(&info.name).unwrap();
                let target = executor
                    .issues()
                    .create_issue(
                        format!("Apply {}", preset.name),
                        String::new(),
                        Priority::Normal,
                        vec![],
                        vec![],
                        None,
                        None,
                        false,
                    )
                    .unwrap()
                    .issue
                    .id;
                executor
                    .gates()
                    .apply_gate_preset(&target, &info.name, None, false, false, &[])
                    .unwrap();
                let attached = executor
                    .storage()
                    .load_issue(&target)
                    .unwrap()
                    .gates_required;
                let expected = preset
                    .gates
                    .iter()
                    .map(|gate| gate.key.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(
                    attached
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>(),
                    expected
                );
                preset
            })
            .collect();
        (listed, shown)
    }

    let memory = TestHarness::new();
    let (_temp, file) = jit::test_utils::setup_test_repo_with_config("").unwrap();
    let presets = ["zeta-checks", "alpha-checks"].map(|name| GatePresetDefinition {
        name: name.into(),
        description: format!("{name} bundle"),
        gates: vec![GateTemplate {
            key: format!("{name}-gate").parse().unwrap(),
            title: format!("{name} gate"),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
        }],
    });
    for preset in &presets {
        let path = VirtualPath::data(
            VirtualPath::GATE_PRESETS
                .relative()
                .as_path()
                .join(format!("{}.json", preset.name)),
        )
        .unwrap();
        let contents = serde_json::to_string(preset).unwrap();
        memory
            .storage
            .add_data_file(path.relative().as_path(), &contents);
        let physical = file.storage().root().join(path.relative().as_path());
        std::fs::create_dir_all(physical.parent().unwrap()).unwrap();
        std::fs::write(physical, contents).unwrap();
    }
    let (memory_list, memory_shown) = read_and_apply(&memory.executor);
    let (file_list, file_shown) = read_and_apply(&file);
    assert_eq!(
        serde_json::to_value(memory_list).unwrap(),
        serde_json::to_value(file_list).unwrap()
    );
    assert_eq!(memory_shown, file_shown);
    let by_name = |presets: Vec<GatePresetDefinition>| {
        presets
            .into_iter()
            .map(|preset| (preset.name.clone(), preset))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(by_name(memory_shown), by_name(presets.to_vec()));
}

/// Define `key` as a manual postcheck gate in the repository's own registry.
fn define_gate(executor: &CommandExecutor<JsonFileStorage>, key: &jit::domain::identity::GateKey) {
    executor
        .gates()
        .define_gate(
            key,
            format!("{key} gate"),
            format!("Repository-declared {key} gate"),
            GateStage::Postcheck,
            GateMode::Manual,
            None,
            100,
        )
        .unwrap_or_else(|error| panic!("define the {key} gate: {error}"));
}

/// Create an issue carrying `gates`, and capture those gates into the
/// project-defined preset `preset`.
fn preset_captured_from_gates(
    executor: &CommandExecutor<JsonFileStorage>,
    preset: &str,
    gates: &[jit::domain::identity::GateKey],
) -> std::path::PathBuf {
    let reference = executor
        .issues()
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
        .expect("create reference issue")
        .issue
        .id;
    for gate in gates {
        define_gate(executor, gate);
        executor.gates().add_gate(&reference, gate).unwrap();
    }
    executor
        .gates()
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
                let temp = declaration_dir(&[("worktree/.jit/config.toml", "")]).unwrap();
                let executor = file_executor(&temp.path().join("worktree")).unwrap();
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
                    &["tests".parse().unwrap()],
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
/// registry appends a registry-scoped audit event per definition write (a
/// created event for a new key, an updated event for a timeout-override
/// overwrite), and a no-write re-application appends none.
#[test]
fn test_apply_gate_preset_appends_definition_events() {
    let temp = declaration_dir(&[("worktree/.jit/config.toml", "")]).unwrap();
    let executor = file_executor(&temp.path().join("worktree")).unwrap();
    // Captured from a definition the project authors, then removed from the
    // registry, so the first application is the write that defines its key.
    preset_captured_from_gates(&executor, "review", &["team-review".parse().unwrap()]);
    executor
        .gates()
        .remove_gate_definition(&"team-review".parse().unwrap())
        .unwrap();

    let issue_with = |title: &str| {
        executor
            .issues()
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
            .issue
            .id
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
        .gates()
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
        .gates()
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
        .gates()
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
    let temp = declaration_dir(&[("worktree/.jit/config.toml", "")]).unwrap();
    let executor = file_executor(&temp.path().join("worktree")).unwrap();

    // SAVE: capture a reference issue's gates into a project-defined preset,
    // written under the project's gate-presets directory.
    let saved_path = preset_captured_from_gates(
        &executor,
        "ci",
        &["tests".parse().unwrap(), "code-review".parse().unwrap()],
    );
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
    let presets = executor.gates().list_gate_presets().expect("list presets");
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
            executor.gates().show_gate_preset(absent).is_err(),
            "{absent} must not resolve as a preset the project never declared"
        );
    }

    // SHOW: the saved preset reports the bundled gate keys.
    let shown = executor
        .gates()
        .show_gate_preset("ci")
        .expect("show preset");
    let keys: Vec<&str> = shown.gates.iter().map(|g| g.key.as_str()).collect();
    assert!(keys.contains(&"tests"), "ci bundles the tests gate");
    assert!(
        keys.contains(&"code-review"),
        "ci bundles the code-review gate"
    );

    // APPLY: attaching the preset to a fresh issue adds both bundled gates.
    let target = executor
        .issues()
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
        .expect("create target issue")
        .issue
        .id;
    let (result, _warnings) = executor
        .gates()
        .apply_gate_preset(&target, "ci", None, false, false, &[])
        .expect("apply project-defined preset");
    assert!(result.added.contains(&"tests".parse().unwrap()));
    assert!(result.added.contains(&"code-review".parse().unwrap()));

    // The target issue now requires both gates.
    let issue = executor.storage().load_issue(&target).expect("load target");
    assert!(issue.gates_required.contains(&"tests".parse().unwrap()));
    assert!(issue
        .gates_required
        .contains(&"code-review".parse().unwrap()));
}
