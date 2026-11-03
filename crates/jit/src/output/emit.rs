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
/// Success writes the success envelope (`--json`) or the response's human
/// rendering to `out` and returns [`ExitCode::Success`]. Findings write the
/// failure envelope with the report as `error.details` (`--json`), or the
/// report's rendering to `out` and one `Error:` diagnostic to `err`, and return
/// the code's [`ErrorCode::exit_code`]. A closed pipe yields
/// [`ExitCode::BrokenPipe`]; the caller owns process exit.
pub fn emit_outcome<R: Response>(
    outcome: &Outcome<R>,
    mode: OutputMode,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> anyhow::Result<ExitCode> {
    let (written, status) = match outcome {
        Outcome::Success(response) if mode.json => (
            writeln!(out, "{}", JsonOutput::success(response).to_json_string()?),
            ExitCode::Success,
        ),
        Outcome::Success(response) => (
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
                .and_then(|()| writeln!(err, "Error: {message}")),
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
    fn test_emit_outcome_closed_pipe_returns_broken_pipe_status() {
        let status = emit_outcome(
            &Outcome::Success(Unit),
            OutputMode::default(),
            &mut ClosedPipe,
            &mut io::sink(),
        )
        .unwrap();
        assert_eq!(status, ExitCode::BrokenPipe);
    }
}
