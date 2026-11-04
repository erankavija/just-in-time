//! Machine-readable failure contract for the server-control command.

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use std::fs;
use std::str::FromStr;

#[test]
fn test_serve_failures_emit_registered_error_envelope() {
    for args in [
        ["serve", "--json"].as_slice(),
        ["serve", "--stop", "--json"].as_slice(),
        ["serve", "--status", "--json"].as_slice(),
    ] {
        let cli = JitCli::initialised().unwrap();
        fs::write(cli.root().join(".jit/server.pid.json"), "{")
            .expect("write malformed server PID file");
        let output = cli
            .command(args)
            .output()
            .expect("run serve failure fixture");

        let stdout = std::str::from_utf8(&output.stdout).expect("stdout is UTF-8");
        let envelope: serde_json::Value =
            serde_json::from_str(stdout).expect("stdout contains exactly one JSON envelope");
        let code = envelope["error"]["code"]
            .as_str()
            .expect("error envelope carries a code");
        let registered = ErrorCode::from_str(code).expect("error code is registered");

        assert_eq!(envelope.as_object().map(|object| object.len()), Some(1));
        assert!(envelope["error"]["message"].is_string());
        assert_eq!(registered, ErrorCode::ParseError);
        assert!(!output.status.success());
    }
}

#[test]
fn test_serve_failure_prints_action_context_once_on_stderr() {
    let cli = JitCli::initialised().unwrap();
    fs::write(cli.root().join(".jit/server.pid.json"), "{").unwrap();
    let output = cli.command(["serve", "--status"]).output().unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("Error: checking server status: "),
        "{stderr}"
    );
    assert_eq!(stderr.matches("checking server status").count(), 1);
    assert_eq!(output.status.code(), Some(1));
}
