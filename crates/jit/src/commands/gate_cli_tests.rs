//! TDD tests for gate CLI commands

#[cfg(test)]
mod tests {
    use crate::commands::test_helpers::setup;
    use crate::declarations::{GateChecker, GateMode, GateStage};
    use std::collections::HashMap;

    // Test: jit gate define <key> --title "Title" --description "Desc" --stage postcheck --mode manual
    #[test]
    fn test_define_manual_gate() {
        let executor = setup();

        let result = executor.gates().define_gate(
            &"manual-review".parse().unwrap(),
            "Manual Review".to_string(),
            "Human code review".to_string(),
            GateStage::Postcheck,
            GateMode::Manual,
            None,
            100,
        );

        assert!(result.is_ok());

        // Verify gate was created
        let gate = executor
            .gates()
            .show_gate_definition(&"manual-review".parse().unwrap())
            .unwrap();
        assert_eq!(gate.key, "manual-review".parse().unwrap());
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
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let result = executor.gates().define_gate(
            &"unit-tests".parse().unwrap(),
            "Unit Tests".to_string(),
            "Run all unit tests".to_string(),
            GateStage::Postcheck,
            GateMode::Auto,
            Some(checker.clone()),
            100,
        );

        assert!(result.is_ok());

        // Verify gate was created with checker
        let gate = executor
            .gates()
            .show_gate_definition(&"unit-tests".parse().unwrap())
            .unwrap();
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
            .gates()
            .define_gate(
                &"test-gate".parse().unwrap(),
                "Test Gate".to_string(),
                "Test".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
            )
            .unwrap();

        // Try to define same gate again
        let result = executor.gates().define_gate(
            &"test-gate".parse().unwrap(),
            "Test Gate 2".to_string(),
            "Test 2".to_string(),
            GateStage::Postcheck,
            GateMode::Manual,
            None,
            100,
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
            .gates()
            .define_gate(
                &"gate-1".parse().unwrap(),
                "Gate 1".to_string(),
                "First gate".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
            )
            .unwrap();

        executor
            .gates()
            .define_gate(
                &"gate-2".parse().unwrap(),
                "Gate 2".to_string(),
                "Second gate".to_string(),
                GateStage::Precheck,
                GateMode::Auto,
                Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                100,
            )
            .unwrap();

        // List gates
        let gates = executor.gates().list_gates().unwrap();
        assert_eq!(gates.len(), 2);

        let keys: Vec<_> = gates.iter().map(|g| &g.key).collect();
        assert!(keys.contains(&&"gate-1".parse().unwrap()));
        assert!(keys.contains(&&"gate-2".parse().unwrap()));
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
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        executor
            .gates()
            .define_gate(
                &"clippy".parse().unwrap(),
                "Clippy Lints".to_string(),
                "No clippy warnings".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(checker),
                100,
            )
            .unwrap();

        // Show gate details
        let gate = executor
            .gates()
            .show_gate_definition(&"clippy".parse().unwrap())
            .unwrap();
        assert_eq!(gate.key, "clippy".parse().unwrap());
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
            ..
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

        let result = executor
            .gates()
            .show_gate_definition(&"nonexistent".parse().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // Test: jit gate remove <key>
    #[test]
    fn test_remove_gate() {
        let executor = setup();

        executor
            .gates()
            .define_gate(
                &"test-gate".parse().unwrap(),
                "Test".to_string(),
                "Test".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
            )
            .unwrap();

        // Verify it exists
        assert!(executor
            .gates()
            .show_gate_definition(&"test-gate".parse().unwrap())
            .is_ok());

        // Remove it
        executor
            .gates()
            .remove_gate_definition(&"test-gate".parse().unwrap())
            .unwrap();

        // Verify it's gone
        assert!(executor
            .gates()
            .show_gate_definition(&"test-gate".parse().unwrap())
            .is_err());
    }

    // Test: Auto gate must have checker
    #[test]
    fn test_auto_gate_requires_checker() {
        let executor = setup();

        let result = executor.gates().define_gate(
            &"auto-no-checker".parse().unwrap(),
            "Auto Gate".to_string(),
            "Test".to_string(),
            GateStage::Postcheck,
            GateMode::Auto,
            None, // No checker for auto gate
            100,
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
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        // Should succeed but checker is ignored for manual gates
        let result = executor.gates().define_gate(
            &"manual-with-checker".parse().unwrap(),
            "Manual".to_string(),
            "Test".to_string(),
            GateStage::Postcheck,
            GateMode::Manual,
            Some(checker),
            100,
        );

        assert!(result.is_ok());

        // Verify checker was ignored
        let gate = executor
            .gates()
            .show_gate_definition(&"manual-with-checker".parse().unwrap())
            .unwrap();
        assert_eq!(gate.mode, GateMode::Manual);
        assert!(
            gate.checker.is_none(),
            "Manual gate should not store checker"
        );
    }
}
