//! The `jit` binary: parse the command line, open the repository, dispatch
//! one command group, render a failure, and exit through the one exit site.

use jit::cli::Cli;
use jit::output::{emit_failure, is_broken_pipe_write_panic, ExitCode, OutputMode, RunStatus};
use std::io;

fn main() {
    // A raw print site panics when a downstream reader closes the pipe; the
    // hook silences that panic, which exits as `ExitCode::BrokenPipe`. Any
    // other panic keeps the default hook and resumes to std's exit 101.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !is_broken_pipe_write_panic(info.payload()) {
            default_hook(info);
        }
    }));
    let status = std::panic::catch_unwind(run).unwrap_or_else(|payload| {
        if is_broken_pipe_write_panic(payload.as_ref()) {
            ExitCode::BrokenPipe.into()
        } else {
            std::panic::resume_unwind(payload)
        }
    });
    std::process::exit(status.code());
}

/// Run one invocation; the opened repository's guards drop before it returns.
fn run() -> RunStatus {
    let args: Vec<String> = std::env::args().collect();
    let cli = match Cli::parse_or_status(&args) {
        Ok(cli) => cli,
        Err(status) => return status,
    };
    let mode = OutputMode {
        json: cli.json,
        quiet: cli.quiet,
    };
    let (mut out, mut err) = (io::stdout(), io::stderr());
    jit::dispatch::bootstrap(&cli)
        .and_then(|repository| jit::dispatch::dispatch(cli, repository, mode, &mut out, &mut err))
        .unwrap_or_else(|error| emit_failure(&error, mode, &mut out, &mut err))
}
