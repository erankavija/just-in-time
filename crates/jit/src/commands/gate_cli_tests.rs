//! TDD tests for gate CLI commands

#[cfg(test)]
mod tests {
    use crate::commands::CommandExecutor;
    use crate::domain::{GateChecker, GateMode, GateStage};
    use crate::storage::{InMemoryStorage, IssueStore};
    use std::collections::HashMap;

    fn setup() -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        CommandExecutor::new(storage)
    }

    // Test: jit gate define <key> --title "Title" --description "Desc" --stage postcheck --mode manual
    #[test]
    fn test_define_manual_gate() {
        let executor = setup();

        let result = executor.define_gate(
            "manual-review".to_string(),
            "Manual Review".to_string(),
            "Human code review".to_string(),
            GateStage::Postcheck,
            GateMode::Manual,
            None,
        );

        assert!(result.is_ok());

        // Verify gate was created
        let gate = executor.show_gate_definition("manual-review").unwrap();
        assert_eq!(gate.key, "manual-review");
        assert_eq!(gate.title, "Manual Review");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Manual);
        assert!(gate.checker.is_none());
    }

    // Test: jit gate define <key> --title "Title" --stage precheck --mode auto --checker-command "cargo test"
    #[test]
    fn test_define_automated_gate_with_checker() {
        let executor = setup();

        let checker = GateChecker::Exec {
            command: "cargo test".to_string(),
            timeout_seconds: 300,
            working_dir: None,
            env: HashMap::new(),
        };

        let result = executor.define_gate(
            "unit-tests".to_string(),
            "Unit Tests".to_string(),
            "Run all unit tests".to_string(),
            GateStage::Postcheck,
            GateMode::Auto,
            Some(checker.clone()),
        );

        assert!(result.is_ok());

        // Verify gate was created with checker
        let gate = executor.show_gate_definition("unit-tests").unwrap();
        assert_eq!(gate.mode, GateMode::Auto);
        assert!(gate.checker.is_some());

        if let Some(GateChecker::Exec { command, .. }) = gate.checker {
            assert_eq!(command, "cargo test");
        }
    }

    // Test: Cannot define duplicate gate
    #[test]
    fn test_define_duplicate_gate_fails() {
        let executor = setup();

        executor
            .define_gate(
                "test-gate".to_string(),
                "Test Gate".to_string(),
                "Test".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
            )
            .unwrap();

        // Try to define same gate again
        let result = executor.define_gate(
            "test-gate".to_string(),
            "Test Gate 2".to_string(),
            "Test 2".to_string(),
            GateStage::Postcheck,
            GateMode::Manual,
            None,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    // Test: jit gate list
    #[test]
    fn test_list_gates() {
        let executor = setup();

        // Define multiple gates
        executor
            .define_gate(
                "gate-1".to_string(),
                "Gate 1".to_string(),
                "First gate".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
            )
            .unwrap();

        executor
            .define_gate(
                "gate-2".to_string(),
                "Gate 2".to_string(),
                "Second gate".to_string(),
                GateStage::Precheck,
                GateMode::Auto,
                Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                }),
            )
            .unwrap();

        // List gates
        let gates = executor.list_gates().unwrap();
        assert_eq!(gates.len(), 2);

        let keys: Vec<_> = gates.iter().map(|g| g.key.as_str()).collect();
        assert!(keys.contains(&"gate-1"));
        assert!(keys.contains(&"gate-2"));
    }

    // Test: jit gate show <key>
    #[test]
    fn test_show_gate_details() {
        let executor = setup();

        let checker = GateChecker::Exec {
            command: "cargo clippy".to_string(),
            timeout_seconds: 180,
            working_dir: Some("src".to_string()),
            env: HashMap::from([("RUST_BACKTRACE".to_string(), "1".to_string())]),
        };

        executor
            .define_gate(
                "clippy".to_string(),
                "Clippy Lints".to_string(),
                "No clippy warnings".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(checker),
            )
            .unwrap();

        // Show gate details
        let gate = executor.show_gate_definition("clippy").unwrap();
        assert_eq!(gate.key, "clippy");
        assert_eq!(gate.title, "Clippy Lints");
        assert_eq!(gate.description, "No clippy warnings");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Auto);

        // Verify checker details
        if let Some(GateChecker::Exec {
            command,
            timeout_seconds,
            working_dir,
            env,
        }) = gate.checker
        {
            assert_eq!(command, "cargo clippy");
            assert_eq!(timeout_seconds, 180);
            assert_eq!(working_dir, Some("src".to_string()));
            assert_eq!(env.get("RUST_BACKTRACE"), Some(&"1".to_string()));
        } else {
            panic!("Expected Exec checker");
        }
    }

    // Test: jit gate show <nonexistent-key>
    #[test]
    fn test_show_nonexistent_gate_fails() {
        let executor = setup();

        let result = executor.show_gate_definition("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // Test: jit gate remove <key>
    #[test]
    fn test_remove_gate() {
        let executor = setup();

        executor
            .define_gate(
                "test-gate".to_string(),
                "Test".to_string(),
                "Test".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
            )
            .unwrap();

        // Verify it exists
        assert!(executor.show_gate_definition("test-gate").is_ok());

        // Remove it
        executor.remove_gate_definition("test-gate").unwrap();

        // Verify it's gone
        assert!(executor.show_gate_definition("test-gate").is_err());
    }

    // Test: Auto gate must have checker
    #[test]
    fn test_auto_gate_requires_checker() {
        let executor = setup();

        let result = executor.define_gate(
            "auto-no-checker".to_string(),
            "Auto Gate".to_string(),
            "Test".to_string(),
            GateStage::Postcheck,
            GateMode::Auto,
            None, // No checker for auto gate
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("checker"));
    }

    // Test: Manual gate should not have checker
    #[test]
    fn test_manual_gate_with_checker_warns() {
        let executor = setup();

        let checker = GateChecker::Exec {
            command: "exit 0".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
        };

        // Should succeed but checker is ignored for manual gates
        let result = executor.define_gate(
            "manual-with-checker".to_string(),
            "Manual".to_string(),
            "Test".to_string(),
            GateStage::Postcheck,
            GateMode::Manual,
            Some(checker),
        );

        assert!(result.is_ok());

        // Verify checker was ignored
        let gate = executor
            .show_gate_definition("manual-with-checker")
            .unwrap();
        assert_eq!(gate.mode, GateMode::Manual);
        assert!(
            gate.checker.is_none(),
            "Manual gate should not store checker"
        );
    }
}
