//! `jit serve` handler.

use super::{CommandContext, Outcome};
use crate::cli::ServeArgs;
use crate::commands::serve::{
    find_web_dir, run_foreground, server_status, start_server, stop_server, ForegroundOptions,
    ForegroundOutcome, ForegroundServerExited, ServeOptions, ServeOutcome, StartupWatch,
    StopOutcome,
};
use crate::output::{ServeResponse, WebUiSource};
use crate::repository_state::RepositoryLayout;
use crate::storage::IssueStore;
use std::path::PathBuf;

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

fn stop(layout: &RepositoryLayout) -> anyhow::Result<ServeResponse> {
    Ok(match stop_server(layout)? {
        StopOutcome::Stopped { pid, port } => ServeResponse::Stopped { pid, port },
        StopOutcome::NotRunning => ServeResponse::NotRunning,
    })
}

fn status(layout: &RepositoryLayout) -> anyhow::Result<ServeResponse> {
    Ok(match server_status(layout)? {
        Some(pf) => ServeResponse::Running {
            pid: pf.pid,
            port: pf.port,
            url: url(pf.port),
            log_file: Some(pf.log_file),
            started_at: Some(pf.started_at),
        },
        None => ServeResponse::NotRunning,
    })
}

fn foreground(
    layout: &RepositoryLayout,
    args: ServeArgs,
    web_dir: Option<PathBuf>,
    json: bool,
) -> anyhow::Result<ServeResponse> {
    let opts = ForegroundOptions {
        layout: layout.clone(),
        preferred_port: args.port,
        web_dir,
        server_binary: None,
    };
    match run_foreground(opts, |launch| {
        if !json {
            let url = url(launch.port);
            println!("Starting server on {url} (foreground, Ctrl+C to stop)");
            println!("  API: {url}/api");
            match &launch.web_dir {
                Some(web) => println!("  Web: {url}/ (from {})", web.display()),
                None => println!("  Web: {url}/ (embedded assets)"),
            }
        }
    })? {
        ForegroundOutcome::AlreadyRunning { pid, port } => Ok(already_running(pid, port)),
        ForegroundOutcome::Exited { port, status } if status.success() => {
            Ok(ServeResponse::Exited {
                port,
                exit_code: status.code(),
            })
        }
        ForegroundOutcome::Exited { port, status } => {
            Err(ForegroundServerExited { port, status }.into())
        }
    }
}

fn daemon(
    layout: &RepositoryLayout,
    args: ServeArgs,
    web_dir: Option<PathBuf>,
) -> anyhow::Result<ServeResponse> {
    let web_dir = web_dir.or_else(find_web_dir);
    let web_ui_source =
        web_dir
            .as_deref()
            .filter(|dir| dir.is_dir())
            .map_or(WebUiSource::Embedded, |dir| WebUiSource::Filesystem {
                dir: dir.display().to_string(),
            });
    let opts = ServeOptions {
        layout: layout.clone(),
        preferred_port: args.port,
        log_file: args.log.map(|log| layout.data_root().join(log)),
        web_dir,
        server_binary: None,
    };
    Ok(match start_server(opts, StartupWatch::for_live_server())? {
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
            web_ui_source,
        },
        ServeOutcome::AlreadyRunning { pid, port } => already_running(pid, port),
    })
}

/// Stop, status, foreground and daemon start share one command; the caller
/// has released its retained mutation session, which the server child's own
/// bootstrap would otherwise deadlock against.
fn serve<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    args: ServeArgs,
) -> anyhow::Result<Outcome<ServeResponse>> {
    let layout = ctx.executor.repository_layout()?;
    let web_dir = args.web_dir.as_ref().map(|dir| {
        let data_root = layout.data_root();
        data_root.parent().unwrap_or(data_root).join(dir)
    });
    let response = if args.stop {
        stop(layout)?
    } else if args.status {
        status(layout)?
    } else if args.fg {
        foreground(layout, args, web_dir, ctx.mode.json)?
    } else {
        daemon(layout, args, web_dir)?
    };
    Ok(Outcome::Success {
        response,
        message: None,
    })
}

command_group! {
    command: ServeArgs,
    storage: [IssueStore],
    leaves: {
        "serve" => args => serve(args) -> ServeResponse,
    }
}
