//! Durability of an automated gate evaluation (`jit:7ca9a0dd`): the gate run's
//! evidence and the issue's recorded verdict are one durable change, read back
//! from a separate process at the boundary an adopter uses.
//!
//! What an interrupted publication leaves behind, and what applying the
//! leftover work converges on, is the publication mechanism's own behaviour,
//! proven once against it in `storage::redo_conformance`
//! (`@/inv/orthogonal-tests`).

use jit::commands::CommandExecutor;
use jit::declarations::{GateChecker, GateMode, GateStage};
use jit::domain::{GateRunStatus, GateStatus, Priority};
use jit::storage::{IssueStore, JsonFileStorage};
use jit::test_cli::JitCli;

#[test]
fn test_check_gate_records_its_run_evidence_with_the_issue_verdict_it_explains() {
    let repo = JitCli::uninitialised().expect("create temporary repository");
    let jit_dir = repo.root().join(".jit");
    let taxonomy = jit::test_taxonomy::test_taxonomy();
    std::fs::create_dir_all(&jit_dir).expect("create repository data directory");
    std::fs::write(
        jit_dir.join("config.toml"),
        format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        ),
    )
    .expect("write repository configuration");

    let storage = JsonFileStorage::new(&jit_dir);
    let layout = jit::storage::discover_repository_layout(repo.root(), storage.root())
        .expect("discover repository layout before initialization");
    let executor = CommandExecutor::new(storage).with_layout(layout);
    executor
        .profiles()
        .initialize_fresh_repository(repo.root(), None)
        .expect("initialize fresh repository");
    executor
        .gates()
        .define_gate(
            &"durability-check".parse().unwrap(),
            "Durability check".to_string(),
            "Durability check".to_string(),
            GateStage::Postcheck,
            GateMode::Auto,
            Some(GateChecker::Exec {
                command: "exit 0".to_string(),
                timeout_seconds: 10,
                working_dir: None,
                env: Default::default(),
                pass_context: false,
                prompt: None,
                prompt_file: None,
            }),
            100,
        )
        .expect("define durability gate");
    let issue_id = executor
        .issues()
        .create_issue(
            "Durability carrier".to_string(),
            "Issue for gate persistence durability regression coverage".to_string(),
            Priority::Normal,
            Vec::new(),
            vec![format!("type:{}", taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
        .expect("create issue requiring durability gate")
        .issue
        .id;
    // Attaching the gate after creation records its explicit `pending` state, so
    // the assertions below read a recorded verdict rather than an absent key.
    executor
        .gates()
        .add_gate(&issue_id, &"durability-check".parse().unwrap())
        .expect("require the durability gate on the issue");

    let evaluated = executor
        .gates()
        .check_gate(&issue_id, &"durability-check".parse().unwrap())
        .expect("the automated gate evaluates");
    assert_eq!(evaluated.status, GateRunStatus::Passed);

    // The evidence and the verdict are one change: a reader that opens the
    // repository afresh finds the run beside the status it explains.
    let durable = JsonFileStorage::new(&jit_dir);
    let recorded = durable
        .load_issue(&issue_id)
        .expect("load the issue the evaluation changed");
    assert_eq!(
        recorded.gates_status[&"durability-check".parse().unwrap()].status,
        GateStatus::Passed,
        "the evaluation's verdict must be durable on the issue"
    );
    let runs = durable
        .list_gate_runs_for_issue(&issue_id)
        .expect("list the gate runs the evaluation recorded");
    assert_eq!(
        runs.iter()
            .filter(|run| run.gate_key.as_str() == "durability-check")
            .map(|run| run.status)
            .collect::<Vec<_>>(),
        vec![GateRunStatus::Passed],
        "the verdict recorded on the issue must be explained by exactly one durable run; \
         runs: {runs:?}"
    );

    // A separate process reading the same repository reports that same pair.
    // `gate status-all` is the CLI's gate-readiness report; what its exit status
    // says about readiness is its own suite's property.
    let output = repo
        .command(["gate", "status-all", issue_id.as_str(), "--json"])
        .output()
        .expect("spawn the jit gate readiness subprocess");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "the readiness report must be one JSON document ({error}); stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let reported = json["gates"]
        .as_array()
        .expect("the readiness report lists every required gate")
        .iter()
        .find(|entry| entry["key"] == "durability-check")
        .expect("the required gate appears in the readiness report");
    assert_eq!(
        reported["status"],
        serde_json::to_value(GateStatus::Passed).expect("a gate status serializes"),
        "the separate process must report the durable verdict; report: {json}"
    );
    assert_eq!(
        json["results"]
            .as_array()
            .expect("the readiness report lists the runs it found")
            .len(),
        1,
        "the separate process must find the run evidence for that verdict; report: {json}"
    );
}
