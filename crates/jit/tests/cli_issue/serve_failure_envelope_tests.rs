//! Machine-readable failure contract for the server-control command.

use jit::output::ErrorCode;
use std::fs;
use std::process::Command;
use std::str::FromStr;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup() -> TempDir {
    let temp = TempDir::new().expect("create temporary repository");
    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .arg("init")
        .output()
        .expect("initialize repository");
    assert!(output.status.success(), "init failed: {output:?}");
    fs::write(temp.path().join(".jit/server.pid.json"), "{")
        .expect("write malformed server PID file");
    temp
}

#[test]
fn test_serve_failures_emit_registered_error_envelope_with_mapped_exit_status() {
    for args in [
        ["serve", "--json"].as_slice(),
        ["serve", "--stop", "--json"].as_slice(),
        ["serve", "--status", "--json"].as_slice(),
    ] {
        let temp = setup();
        let output = Command::new(jit_binary())
            .current_dir(&temp)
            .args(args)
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
        assert_eq!(registered, ErrorCode::GenericError);
        assert_eq!(output.status.code(), Some(registered.exit_code().code()));
    }
}
