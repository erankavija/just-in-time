//! Tests for gate run result storage

#[cfg(test)]
mod tests {
    use crate::declarations::GateStage;
    use crate::domain::gate_findings::{GateFinding, GateFindings};
    use crate::domain::{GateRunResult, GateRunStatus};
    use crate::repository_state::{
        finalize, CaptureBudget, CaptureSpec, MutationContext, MutationIntent, VirtualPath,
    };
    use crate::storage::{
        discover_repository_layout, IssueStore, JsonFileStorage, RepositoryStateStore,
    };
    use chrono::Utc;
    use tempfile::TempDir;

    fn record_gate_run(
        temp: &TempDir,
        storage: &JsonFileStorage,
        draft: GateRunResult,
        seed: u8,
    ) -> crate::domain::identity::GateRunId {
        let layout = discover_repository_layout(temp.path(), storage.root()).unwrap();
        let context = MutationContext::deterministic([seed; 32], draft.started_at);
        let run_id = draft.run_id.clone();
        let spec = CaptureSpec::new(
            [
                VirtualPath::GATE_RUNS,
                VirtualPath::gate_run_issue_directory(&draft.issue_id),
                VirtualPath::gate_run_record(&draft.issue_id, &run_id),
            ],
            CaptureBudget {
                max_listings: 0,
                max_bytes: 1024 * 1024,
                max_depth: 3,
            },
        )
        .unwrap();
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(spec).unwrap();
        let plan = finalize(
            &layout,
            &image,
            &context,
            &[MutationIntent::RecordGateRun {
                draft: Box::new(draft),
            }],
            None,
        )
        .unwrap();
        session.apply(&plan).unwrap();
        run_id
    }

    #[test]
    fn test_record_and_load_gate_run_result() {
        let (temp, storage) = crate::test_utils::setup_test_repo().unwrap();

        let result = GateRunResult {
            schema_version: 1,
            run_id: crate::test_utils::gate_run_id(1),
            gate_key: "unit-tests".parse().unwrap(),
            stage: GateStage::Postcheck,
            issue_id: crate::test_utils::issue_id(51),
            commit: Some("abc123".to_string()),
            branch: Some("main".to_string()),
            status: GateRunStatus::Passed,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            duration_ms: Some(1500),
            exit_code: Some(0),
            stdout: "All tests passed".to_string(),
            stderr: "".to_string(),
            command: "cargo test".to_string(),
            by: Some(crate::domain::Assignee::AUTO_EXECUTOR),
            message: None,
            findings: Some(GateFindings {
                verdict: "pass".to_string(),
                summary: "all green".to_string(),
                findings: vec![GateFinding {
                    id: "F1".to_string(),
                    severity: "low".to_string(),
                    disposition: Some("advisory".to_string()),
                    origin: Some("pre-existing".to_string()),
                    summary: "nit".to_string(),
                    file: Some("src/x.rs".to_string()),
                    line: Some(7),
                    references: vec![
                        "@/inv/atomic-writes".to_string(),
                        "checker:opaque value".to_string(),
                    ],
                }],
            }),
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
            evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        };

        let issue_id = result.issue_id.clone();
        let run_id = record_gate_run(&temp, &storage, result, 1);

        // Load result
        let loaded = storage.load_gate_run_result(&issue_id, &run_id).unwrap();
        assert_eq!(loaded.run_id, run_id);
        assert_eq!(loaded.gate_key.as_str(), "unit-tests");
        assert_eq!(loaded.status, GateRunStatus::Passed);
        assert_eq!(loaded.exit_code, Some(0));
        // Structured findings survive the persistence round-trip.
        let findings = loaded.findings.expect("findings should persist");
        assert_eq!(findings.verdict, "pass");
        assert_eq!(findings.findings.len(), 1);
        assert_eq!(findings.findings[0].id, "F1");
        assert_eq!(
            findings.findings[0].disposition.as_deref(),
            Some("advisory")
        );
        assert_eq!(findings.findings[0].origin.as_deref(), Some("pre-existing"));
        assert_eq!(findings.findings[0].line, Some(7));
        assert_eq!(
            findings.findings[0].references,
            ["@/inv/atomic-writes", "checker:opaque value"]
        );
    }

    #[test]
    fn test_load_gate_run_without_findings_field_defaults_to_none() {
        // A run recorded before the findings field existed must still load: the
        // serde default fills `findings` with None rather than erroring.
        let legacy = serde_json::json!({
            "schema_version": 1,
            "run_id": crate::test_utils::gate_run_id(3),
            "gate_key": "tests",
            "stage": "postcheck",
            "issue_id": crate::test_utils::issue_id(1),
            "commit": null,
            "branch": null,
            "status": "passed",
            "started_at": Utc::now().to_rfc3339(),
            "completed_at": null,
            "duration_ms": null,
            "exit_code": 0,
            "stdout": "ok",
            "stderr": "",
            "command": "true",
            "by": null,
            "message": null
        });
        let loaded: GateRunResult = serde_json::from_value(legacy).unwrap();
        assert!(loaded.findings.is_none());
    }

    /// A run recorded while gate runs carried the retired `tree_dirty` key still
    /// loads; the key is ignored.
    #[test]
    fn test_load_gate_run_ignores_retired_tree_dirty_key() {
        let base = serde_json::json!({
            "schema_version": 1,
            "run_id": crate::test_utils::gate_run_id(4),
            "gate_key": "tests",
            "stage": "postcheck",
            "issue_id": crate::test_utils::issue_id(1),
            "commit": "abc123",
            "branch": "main",
            "status": "passed",
            "started_at": Utc::now().to_rfc3339(),
            "completed_at": null,
            "duration_ms": null,
            "exit_code": 0,
            "stdout": "ok",
            "stderr": "",
            "command": "true",
            "by": null,
            "message": null,
            "tree_dirty": true
        });

        let loaded: GateRunResult = serde_json::from_value(base).unwrap();
        assert_eq!(loaded.run_id, crate::test_utils::gate_run_id(4));
    }

    #[test]
    fn test_load_legacy_gate_finding_without_references_defaults_empty() {
        let legacy = serde_json::json!({
            "schema_version": 1,
            "run_id": crate::test_utils::gate_run_id(5),
            "gate_key": "review",
            "stage": "postcheck",
            "issue_id": crate::test_utils::issue_id(1),
            "commit": null,
            "branch": null,
            "status": "failed",
            "started_at": Utc::now().to_rfc3339(),
            "completed_at": null,
            "duration_ms": null,
            "exit_code": 1,
            "stdout": "review",
            "stderr": "",
            "command": "review",
            "by": null,
            "message": null,
            "findings": {
                "verdict": "fail",
                "summary": "one defect",
                "findings": [{"id": "F1", "severity": "high", "summary": "legacy"}]
            }
        });

        let loaded: GateRunResult = serde_json::from_value(legacy).unwrap();

        assert!(loaded.findings.unwrap().findings[0].references.is_empty());
    }

    #[test]
    fn test_gate_run_with_no_git_context() {
        let (temp, storage) = crate::test_utils::setup_test_repo().unwrap();

        let result = GateRunResult {
            schema_version: 1,
            run_id: crate::test_utils::gate_run_id(2),
            gate_key: "lint".parse().unwrap(),
            stage: GateStage::Postcheck,
            issue_id: crate::test_utils::issue_id(52),
            commit: None, // No git context
            branch: None, // No git context
            status: GateRunStatus::Passed,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            duration_ms: Some(500),
            exit_code: Some(0),
            stdout: "Linting passed".to_string(),
            stderr: "".to_string(),
            command: "cargo clippy".to_string(),
            by: Some(crate::domain::Assignee::AUTO_EXECUTOR),
            message: None,
            findings: None,
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
            evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        };

        let issue_id = result.issue_id.clone();
        let run_id = record_gate_run(&temp, &storage, result, 1);
        let loaded = storage.load_gate_run_result(&issue_id, &run_id).unwrap();

        assert!(loaded.commit.is_none());
        assert!(loaded.branch.is_none());
    }
}
