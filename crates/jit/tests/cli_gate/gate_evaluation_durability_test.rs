//! Regression coverage for the durability of an automated gate evaluation
//! (`jit:7ca9a0dd`). The test mixes in-process injection with a subprocess
//! assertion because the command subprocess exposes no failure injector, so
//! the failure is injected in process and the recovered repository is then
//! inspected from a separate process at the boundary an adopter uses.

use jit::commands::CommandExecutor;
use jit::declarations::{GateChecker, GateMode, GateStage};
use jit::domain::{GateStatus, Priority};
use jit::storage::{
    IssueStore, JsonFileStorage, RepositoryStateStore, TransactionFailureInjector,
    TransactionFailurePoint,
};
use std::process::Command;
use std::sync::Arc;

/// Fails the transaction kernel at the commit decision — the point where the
/// coupled issue/gate-run/event plan has staged and mutated its targets but has
/// not committed, so recovery must roll every part of it back together.
struct FailAtCommitDecision;

impl TransactionFailureInjector for FailAtCommitDecision {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &TransactionFailurePoint::RepositoryBeforeCommitDecision {
            return Err(std::io::Error::other("injected gate persistence failure"));
        }
        Ok(())
    }
}

#[test]
fn test_check_gate_persistence_failure_leaves_no_durable_gate_evidence() {
    let repo = tempfile::TempDir::new().expect("create temporary repository");
    let jit_dir = repo.path().join(".jit");
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
    let layout = jit::storage::discover_repository_layout(repo.path(), storage.root())
        .expect("discover repository layout before initialization");
    let executor = CommandExecutor::new(storage).with_layout(layout);
    executor
        .initialize_fresh_repository(repo.path(), None)
        .expect("initialize fresh repository");
    let layout = jit::storage::discover_repository_layout(repo.path(), jit_dir.as_path())
        .expect("rediscover repository layout after initialization");

    executor
        .define_gate(
            "durability-check".to_string(),
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
            None,
        )
        .expect("define durability gate");
    let (issue_id, _) = executor
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
        .expect("create issue requiring durability gate");
    // Attaching the gate after creation records its explicit `pending` state, so
    // the assertions below read a recorded verdict rather than an absent key.
    executor
        .add_gate(&issue_id, "durability-check".to_string())
        .expect("require the durability gate on the issue");

    // REQ-01: an injected persistence failure makes check_gate return Err.
    let injected =
        JsonFileStorage::with_repository_state_failures(&jit_dir, Arc::new(FailAtCommitDecision));
    let injected_executor = CommandExecutor::new(injected).with_layout(layout.clone());
    let error = injected_executor
        .check_gate(&issue_id, "durability-check")
        .expect_err("an injected persistence failure must not be reported as a passing gate");
    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("injected gate persistence failure"),
        "the surfaced error must be the injected persistence failure; error: {error_text}"
    );

    // REQ-02: recovery leaves the gate pending and records no failed run.
    let recovered = JsonFileStorage::new(&jit_dir);
    let recovered_layout = jit::storage::discover_repository_layout(repo.path(), recovered.root())
        .expect("rediscover repository layout for recovery");
    drop(
        recovered
            .open_mutation_session(recovered_layout)
            .expect("recovery converges"),
    );
    let recovered_issue = recovered
        .load_issue(&issue_id)
        .expect("load issue after persistence recovery");
    let gate_status = recovered_issue
        .gates_status
        .get("durability-check")
        .map(|state| state.status)
        .expect("a required gate always carries a recorded status");
    assert_eq!(
        gate_status,
        GateStatus::Pending,
        "a failed persistence must leave no recorded gate verdict"
    );
    let gate_runs = recovered
        .list_gate_runs_for_issue(&issue_id)
        .expect("list gate runs after persistence recovery");
    assert!(
        gate_runs.is_empty(),
        "a failed persistence must leave no gate-run record; runs: {gate_runs:?}"
    );

    // REQ-03: a separate process reading the same repository reports that same
    // gate status. `gate status-all` is the CLI's gate-readiness report, and it
    // refuses to call an unpassed gate ready by exiting ValidationFailed.
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo.path())
        .args(["gate", "status-all", &issue_id, "--json"])
        .output()
        .expect("spawn the jit gate readiness subprocess");
    assert_eq!(
        output.status.code(),
        Some(jit::output::ExitCode::ValidationFailed.code()),
        "a separate process must not report the unpassed gate as ready; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse the gate readiness report");
    let reported = json["gates"]
        .as_array()
        .expect("the readiness report lists every required gate")
        .iter()
        .find(|entry| entry["key"] == "durability-check")
        .expect("the required gate appears in the readiness report");
    assert_eq!(
        reported["status"],
        serde_json::to_value(GateStatus::Pending).expect("a gate status serializes"),
        "the separate process must report the same non-passed status; report: {json}"
    );
    assert!(
        json["results"]
            .as_array()
            .is_some_and(|runs| runs.is_empty()),
        "the separate process must find no gate-run evidence for the failed attempt; report: {json}"
    );
}
