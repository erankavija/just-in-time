//! The one renderer of a handler [`Outcome`](crate::dispatch::Outcome).

use super::{ErrorCode, ExitCode, JsonError, JsonOutput, OutputMode};
use crate::dispatch::Outcome;
use schemars::JsonSchema;
use serde::Serialize;
use std::io::{self, Write};

/// The human rendering of a response or report: the complete text, trailing
/// newline included, written verbatim to stdout.
pub trait HumanRender {
    fn render_human(&self) -> String;
}

/// A handler's success response: its JSON shape, published schema and human
/// rendering.
pub trait Response: Serialize + JsonSchema + HumanRender {}

impl<T: Serialize + JsonSchema + HumanRender> Response for T {}

/// A findings report, carried type-erased by [`Outcome::Findings`].
pub trait Report: HumanRender {
    /// The report as the failure envelope's `error.details`.
    fn details(&self) -> serde_json::Result<serde_json::Value>;
}

impl<T: Serialize + HumanRender> Report for T {
    fn details(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::to_value(self)
    }
}

/// Render `outcome` under `mode` and return the process exit status.
///
/// Success writes the success envelope with the outcome's `message`
/// (`--json`) or the response's human rendering to `out` and returns
/// [`ExitCode::Success`]. Findings write the failure envelope with the report
/// as `error.details` (`--json`), or the report's rendering to `out` and the
/// outcome's `message` as one unprefixed line to `err`, and return the code's [`ErrorCode::exit_code`]. A closed pipe yields
/// [`ExitCode::BrokenPipe`]; the caller owns process exit.
pub fn emit_outcome<R: Response>(
    outcome: &Outcome<R>,
    mode: OutputMode,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> anyhow::Result<ExitCode> {
    let (written, status) = match outcome {
        Outcome::Success { response, message } if mode.json => {
            let envelope = JsonOutput {
                data: response,
                message: message.clone(),
            };
            (
                writeln!(out, "{}", envelope.to_json_string()?),
                ExitCode::Success,
            )
        }
        Outcome::Success { response, .. } => (
            out.write_all(response.render_human().as_bytes()),
            ExitCode::Success,
        ),
        Outcome::Findings {
            code,
            message,
            report,
        } if mode.json => {
            let envelope = JsonError::new(*code, message.as_str()).with_details(report.details()?);
            (
                writeln!(out, "{}", envelope.to_json_string()?),
                envelope.exit_code(),
            )
        }
        Outcome::Findings {
            code,
            message,
            report,
        } => (
            out.write_all(report.render_human().as_bytes())
                .and_then(|()| writeln!(err, "{message}")),
            ErrorCode::exit_code(*code),
        ),
    };
    match written {
        Ok(()) => Ok(status),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(ExitCode::BrokenPipe),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, JsonSchema)]
    struct Unit;

    impl HumanRender for Unit {
        fn render_human(&self) -> String {
            "unit\n".to_owned()
        }
    }

    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_emit_outcome_success_json_carries_outcome_message_beside_response() {
        #[derive(Serialize, JsonSchema)]
        struct Named {
            name: &'static str,
        }
        impl HumanRender for Named {
            fn render_human(&self) -> String {
                String::new()
            }
        }
        let mut out = Vec::new();

        emit_outcome(
            &Outcome::Success {
                response: Named { name: "n" },
                message: Some("done".to_owned()),
            },
            OutputMode {
                json: true,
                quiet: false,
            },
            &mut out,
            &mut io::sink(),
        )
        .unwrap();

        let printed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(printed["message"], "done");
        assert_eq!(printed["name"], "n");
    }

    #[test]
    fn test_emit_outcome_closed_pipe_returns_broken_pipe_status() {
        let status = emit_outcome(
            &Outcome::Success {
                response: Unit,
                message: None,
            },
            OutputMode::default(),
            &mut ClosedPipe,
            &mut io::sink(),
        )
        .unwrap();
        assert_eq!(status, ExitCode::BrokenPipe);
    }
}
