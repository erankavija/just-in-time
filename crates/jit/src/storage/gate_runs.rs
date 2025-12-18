//! Tests for gate run result storage

#[cfg(test)]
mod tests {
    use crate::domain::{GateRunResult, GateRunStatus, GateStage};
    use crate::storage::{IssueStore, JsonFileStorage};
    use chrono::Utc;
    use tempfile::TempDir;

    fn setup_storage() -> (TempDir, JsonFileStorage) {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().to_path_buf());
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
            status: GateRunStatus::Passed,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            duration_ms: Some(1500),
            exit_code: Some(0),
            stdout: "All tests passed".to_string(),
            stderr: "".to_string(),
            command: "cargo test".to_string(),
            by: Some("auto:executor".to_string()),
            message: None,
        };

        // Save result
        storage.save_gate_run_result(&result).unwrap();

        // Load result
        let loaded = storage.load_gate_run_result("test-run-1").unwrap();
        assert_eq!(loaded.run_id, "test-run-1");
        assert_eq!(loaded.gate_key, "unit-tests");
        assert_eq!(loaded.status, GateRunStatus::Passed);
        assert_eq!(loaded.exit_code, Some(0));
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
                by: Some("auto:executor".to_string()),
                message: None,
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
            by: Some("auto:executor".to_string()),
            message: None,
        };

        storage.save_gate_run_result(&result).unwrap();
        let loaded = storage.load_gate_run_result("test-run-nogit").unwrap();

        assert!(loaded.commit.is_none());
        assert!(loaded.branch.is_none());
    }
}
