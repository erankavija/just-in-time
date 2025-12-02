//! Output helper macros for reducing JSON boilerplate in main.rs

/// Output a simple message: either JSON-wrapped or plain text
/// 
/// # Examples
/// 
/// ```ignore
/// output_message!(json, "Created issue: {}", id);
/// ```
#[macro_export]
macro_rules! output_message {
    ($json:expr, $($arg:tt)*) => {
        if $json {
            use output::JsonOutput;
            use serde_json::json;
            let msg = format!($($arg)*);
            let output = JsonOutput::success(json!({"message": msg}));
            println!("{}", output.to_json_string()?);
        } else {
            println!($($arg)*);
        }
    };
}

/// Output data as JSON or custom human-readable format
///
/// # Examples
///
/// ```ignore
/// output_data!(json, issue, {
///     println!("ID: {}", issue.id);
///     println!("Title: {}", issue.title);
/// });
/// ```
#[macro_export]
macro_rules! output_data {
    ($json:expr, $data:expr, $human_block:block) => {
        if $json {
            use output::JsonOutput;
            let output = JsonOutput::success(&$data);
            println!("{}", output.to_json_string()?);
        } else {
            $human_block
        }
    };
}

/// Output structured JSON data with custom human formatting
///
/// # Examples
///
/// ```ignore
/// output_json!(json, json!({
///     "issues": issues,
///     "count": issues.len()
/// }), {
///     println!("Found {} issues", issues.len());
/// });
/// ```
#[macro_export]
macro_rules! output_json {
    ($json:expr, $json_data:expr, $human_block:block) => {
        if $json {
            use output::JsonOutput;
            let output = JsonOutput::success($json_data);
            println!("{}", output.to_json_string()?);
        } else {
            $human_block
        }
    };
}

/// Handle error with appropriate exit code when using --json
///
/// # Examples
///
/// ```ignore
/// match executor.show_issue(&id) {
///     Ok(issue) => output_data!(json, issue, { /* human format */ }),
///     Err(e) => handle_json_error!(json, e, JsonError::issue_not_found(&id)),
/// }
/// ```
#[macro_export]
macro_rules! handle_json_error {
    ($json:expr, $err:expr, $json_error:expr) => {
        if $json {
            println!("{}", $json_error.to_json_string()?);
            std::process::exit($json_error.exit_code().code());
        } else {
            return Err($err);
        }
    };
}
