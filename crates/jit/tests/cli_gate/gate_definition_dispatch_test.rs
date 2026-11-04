//! Gate definition dispatch preserves typed failures across output modes.

use jit::test_cli::JitCli;
use serde_json::Value;

#[test]
fn test_gate_define_invalid_env_reports_invalid_argument_in_both_modes() {
    let cli = JitCli::initialised().unwrap();
    for json in [false, true] {
        let mut args = vec![
            "gate",
            "define",
            "example",
            "--title",
            "Example",
            "--description",
            "Example",
            "--env",
            "missing-equals",
        ];
        if json {
            args.push("--json");
        }
        let output = cli.command(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        if json {
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["error"]["code"], "INVALID_ARGUMENT");
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("expected KEY=VALUE"));
        }
    }
}

#[test]
fn test_gate_define_validation_failure_uses_classified_code_in_both_modes() {
    let cli = JitCli::initialised().unwrap();
    for json in [false, true] {
        let mut args = vec![
            "gate",
            "define",
            "example",
            "--title",
            "Example",
            "--description",
            "Example",
            "--auto",
        ];
        if json {
            args.push("--json");
        }
        let output = cli.command(args).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        if json {
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["error"]["code"], "GENERIC_ERROR");
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("checker"));
        }
    }
}
