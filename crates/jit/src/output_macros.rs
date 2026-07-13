//! Output helper macros for reducing JSON boilerplate in main.rs

/// Output a simple message: either JSON-wrapped or plain text
/// Respects --quiet flag to suppress non-essential output
#[macro_export]
macro_rules! output_message {
    ($quiet:expr, $json:expr, $command:expr, $($arg:tt)*) => {
        if $json {
            use jit::output::JsonOutput;
            use serde_json::json;
            let msg = format!($($arg)*);
            let output = JsonOutput::success(json!({"message": msg}), $command);
            println!("{}", output.to_json_string()?);
        } else if !$quiet {
            println!($($arg)*);
        }
    };
}

/// Output data as JSON or custom human-readable format
/// Data output is preserved in quiet mode (essential output)
#[macro_export]
macro_rules! output_data {
    ($quiet:expr, $json:expr, $command:expr, $data:expr, $msg:expr, $human_block:block) => {
        if $json {
            use jit::output::JsonOutput;
            let output = JsonOutput::success(&$data, $command).with_message($msg);
            println!("{}", output.to_json_string()?);
        } else {
            $human_block
        }
    };
    ($quiet:expr, $json:expr, $command:expr, $data:expr, $human_block:block) => {
        if $json {
            use jit::output::JsonOutput;
            let output = JsonOutput::success(&$data, $command);
            println!("{}", output.to_json_string()?);
        } else {
            $human_block
        }
    };
}

/// Output structured JSON data with custom human formatting
/// Human output preserved in quiet mode (essential data)
#[macro_export]
macro_rules! output_json {
    ($quiet:expr, $json:expr, $command:expr, $json_data:expr, $msg:expr, $human_block:block) => {
        if $json {
            use jit::output::JsonOutput;
            let output = JsonOutput::success($json_data, $command).with_message($msg);
            println!("{}", output.to_json_string()?);
        } else {
            $human_block
        }
    };
    ($quiet:expr, $json:expr, $command:expr, $json_data:expr, $human_block:block) => {
        if $json {
            use jit::output::JsonOutput;
            let output = JsonOutput::success($json_data, $command);
            println!("{}", output.to_json_string()?);
        } else {
            $human_block
        }
    };
}

/// Handle error with appropriate exit code when using --json
#[macro_export]
macro_rules! handle_json_error {
    ($json:expr, $err:expr, $json_error:expr) => {
        if $json {
            // Refine the call-site fallback when the underlying failure is a typed
            // id-resolution error (ambiguous / too-short prefix), so those carry
            // their distinguishing code and exit 2 at every call site.
            let json_error = jit::output::refine_id_error(&$err, $json_error);
            println!("{}", json_error.to_json_string()?);
            std::process::exit(json_error.exit_code().code());
        } else {
            return Err($err);
        }
    };
}
