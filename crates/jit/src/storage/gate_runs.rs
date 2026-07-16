//! Tests for gate run result storage

#[cfg(test)]
mod tests {
    use crate::domain::{GateFinding, GateFindings, GateRunResult, GateRunStatus, GateStage};
    use crate::storage::{IssueStore, JsonFileStorage};
    use chrono::Utc;
    use tempfile::TempDir;

    fn setup_storage() -> (TempDir, JsonFileStorage) {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path());
        (temp, storage)
    }

    #[test]
    fn test_save_and_load_gate_run_result() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let result = GateRunResult {
            schema_version: 1,
            run_id: "test-run-1".to_string(),
            gate_key: "unit-tests".to_string(),
            stage: GateStage::Postcheck,
            issue_id: "issue-123".to_string(),
            commit: Some("abc123".to_string()),
            branch: Some("main".to_string()),
            tree_dirty: None,
            status: GateRunStatus::Passed,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            duration_ms: Some(1500),
            exit_code: Some(0),
            stdout: "All tests passed".to_string(),
            stderr: "".to_string(),
            command: "cargo test".to_string(),
            by: Some(crate::gate_execution::AUTO_EXECUTOR.to_string()),
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
        };

        // Save result
        storage.save_gate_run_result(&result).unwrap();

        // Load result
        let loaded = storage.load_gate_run_result("test-run-1").unwrap();
        assert_eq!(loaded.run_id, "test-run-1");
        assert_eq!(loaded.gate_key, "unit-tests");
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
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let legacy = serde_json::json!({
            "schema_version": 1,
            "run_id": "legacy-run",
            "gate_key": "tests",
            "stage": "postcheck",
            "issue_id": "issue-legacy",
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

    /// REQ-03: a run recorded before the `tree_dirty` field existed must still
    /// load; the serde default fills it with `None` rather than erroring, and a
    /// record that carries the field round-trips its value.
    #[test]
    fn test_load_gate_run_tree_dirty_defaults_and_round_trips() {
        let base = serde_json::json!({
            "schema_version": 1,
            "run_id": "tree-legacy",
            "gate_key": "tests",
            "stage": "postcheck",
            "issue_id": "issue-legacy",
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
            "message": null
        });

        // No `tree_dirty` key: defaults to None.
        let legacy: GateRunResult = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(legacy.tree_dirty, None);

        // Present and true: preserved through load.
        let mut dirty = base;
        dirty["tree_dirty"] = serde_json::json!(true);
        let loaded: GateRunResult = serde_json::from_value(dirty).unwrap();
        assert_eq!(loaded.tree_dirty, Some(true));
    }

    #[test]
    fn test_load_legacy_gate_finding_without_references_defaults_empty() {
        let legacy = serde_json::json!({
            "schema_version": 1,
            "run_id": "legacy-finding-run",
            "gate_key": "review",
            "stage": "postcheck",
            "issue_id": "issue-legacy",
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
    fn test_list_gate_runs_for_issue() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        // Create multiple runs for the same issue
        for i in 0..3 {
            let result = GateRunResult {
                schema_version: 1,
                run_id: format!("run-{}", i),
                gate_key: "unit-tests".to_string(),
                stage: GateStage::Postcheck,
                issue_id: "issue-123".to_string(),
                commit: None,
                branch: None,
                tree_dirty: None,
                status: if i == 2 {
                    GateRunStatus::Passed
                } else {
                    GateRunStatus::Failed
                },
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
                duration_ms: Some(1000),
                exit_code: Some(if i == 2 { 0 } else { 1 }),
                stdout: format!("Output {}", i),
                stderr: "".to_string(),
                command: "cargo test".to_string(),
                by: Some(crate::gate_execution::AUTO_EXECUTOR.to_string()),
                message: None,
                findings: None,
            };
            storage.save_gate_run_result(&result).unwrap();
        }

        // List runs for issue
        let runs = storage.list_gate_runs_for_issue("issue-123").unwrap();
        assert_eq!(runs.len(), 3);

        // Verify we got all runs
        let run_ids: Vec<_> = runs.iter().map(|r| r.run_id.as_str()).collect();
        assert!(run_ids.contains(&"run-0"));
        assert!(run_ids.contains(&"run-1"));
        assert!(run_ids.contains(&"run-2"));
    }

    #[test]
    fn test_list_gate_runs_for_nonexistent_issue() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let runs = storage.list_gate_runs_for_issue("nonexistent").unwrap();
        assert_eq!(runs.len(), 0);
    }

    #[test]
    fn test_load_nonexistent_gate_run() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let result = storage.load_gate_run_result("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_gate_run_with_no_git_context() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let result = GateRunResult {
            schema_version: 1,
            run_id: "test-run-nogit".to_string(),
            gate_key: "lint".to_string(),
            stage: GateStage::Postcheck,
            issue_id: "issue-456".to_string(),
            commit: None,     // No git context
            branch: None,     // No git context
            tree_dirty: None, // No git context
            status: GateRunStatus::Passed,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            duration_ms: Some(500),
            exit_code: Some(0),
            stdout: "Linting passed".to_string(),
            stderr: "".to_string(),
            command: "cargo clippy".to_string(),
            by: Some(crate::gate_execution::AUTO_EXECUTOR.to_string()),
            message: None,
            findings: None,
        };

        storage.save_gate_run_result(&result).unwrap();
        let loaded = storage.load_gate_run_result("test-run-nogit").unwrap();

        assert!(loaded.commit.is_none());
        assert!(loaded.branch.is_none());
    }
}
