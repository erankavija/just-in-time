//! `jit serve` handler.

use super::{CommandContext, Outcome};
use crate::cli::ServeArgs;
use crate::commands::serve::{
    find_web_dir, run_foreground, server_status, start_server, stop_server, ForegroundOptions,
    ForegroundOutcome, ForegroundServerExited, ServeActionFailed, ServeOptions, ServeOutcome,
    StartupWatch, StopOutcome,
};
use crate::output::{ServeResponse, WebUiSource};
use crate::storage::IssueStore;

fn url(port: u16) -> String {
    format!("http://localhost:{port}")
}

fn already_running(pid: u32, port: u16) -> ServeResponse {
    ServeResponse::Running {
        pid,
        port,
        url: url(port),
        log_file: None,
        started_at: None,
    }
}

/// Stop, status, foreground and daemon start share one command. The caller has
/// released its retained mutation session, which the server child's own
/// bootstrap would otherwise deadlock against.
fn serve<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    args: ServeArgs,
) -> anyhow::Result<Outcome<ServeResponse>> {
    let layout = ctx.executor.repository_layout()?;
    let data_root = layout.data_root();
    let web_dir = args
        .web_dir
        .map(|dir| data_root.parent().unwrap_or(data_root).join(dir));
    let response = if args.stop {
        match stop_server(layout).map_err(ServeActionFailed::during("stopping server"))? {
            StopOutcome::Stopped { pid, port } => ServeResponse::Stopped { pid, port },
            StopOutcome::NotRunning => ServeResponse::NotRunning,
        }
    } else if args.status {
        server_status(layout)
            .map_err(ServeActionFailed::during("checking server status"))?
            .map_or(ServeResponse::NotRunning, |pf| ServeResponse::Running {
                pid: pf.pid,
                port: pf.port,
                url: url(pf.port),
                log_file: Some(pf.log_file),
                started_at: Some(pf.started_at),
            })
    } else if args.fg {
        let opts = ForegroundOptions {
            layout: layout.clone(),
            preferred_port: args.port,
            web_dir,
            server_binary: None,
        };
        match run_foreground(opts, |launch| {
            // Named exception D-02: banner streams before the child runs.
            if !ctx.mode.json {
                let url = url(launch.port);
                println!("Starting server on {url} (foreground, Ctrl+C to stop)");
                println!("  API: {url}/api");
                match &launch.web_dir {
                    Some(web) => println!("  Web: {url}/ (from {})", web.display()),
                    None => println!("  Web: {url}/ (embedded assets)"),
                }
            }
        })? {
            ForegroundOutcome::AlreadyRunning { pid, port } => already_running(pid, port),
            ForegroundOutcome::Exited { port, status } if status.success() => {
                ServeResponse::Exited {
                    port,
                    exit_code: status.code(),
                }
            }
            ForegroundOutcome::Exited { port, status } => {
                return Err(ForegroundServerExited { port, status }.into())
            }
        }
    } else {
        let web_dir = web_dir.or_else(find_web_dir);
        let shown_dir = web_dir
            .as_deref()
            .filter(|dir| dir.is_dir())
            .map(|dir| dir.display().to_string());
        let opts = ServeOptions {
            layout: layout.clone(),
            preferred_port: args.port,
            log_file: args.log.map(|log| data_root.join(log)),
            web_dir,
            server_binary: None,
        };
        match start_server(opts, StartupWatch::for_live_server())
            .map_err(ServeActionFailed::during("starting server"))?
        {
            ServeOutcome::Started {
                pid,
                port,
                log_file,
            } => ServeResponse::Started {
                pid,
                port,
                url: url(port),
                log_file,
                web_ui: true,
                web_ui_source: if shown_dir.is_some() {
                    WebUiSource::Filesystem
                } else {
                    WebUiSource::Embedded
                },
                web_dir: shown_dir,
            },
            ServeOutcome::AlreadyRunning { pid, port } => already_running(pid, port),
        }
    };
    Ok(Outcome::success(response))
}

command_group! {
    command: ServeArgs,
    storage: [IssueStore],
    leaves: {
        "serve" => args => serve(args) -> ServeResponse,
    }
}
