//! JIT REST API Server
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

#![deny(unsafe_code)]

#[cfg(feature = "embed-web")]
mod embedded;
mod routes;
mod sse;
mod watcher;

use anyhow::{Context, Result};
use axum::Router;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

use jit::commands::CommandExecutor;
use jit_server::{prepare_server_storage, resolve_listener, ListenerSource};
use routes::AppState;

/// JIT REST API Server
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to JIT repository (.jit directory)
    ///
    /// Can also be set via JIT_DATA_DIR environment variable.
    /// Defaults to ./.jit if not specified.
    #[arg(short, long, env = "JIT_DATA_DIR", default_value = ".jit")]
    data_dir: String,

    /// Address to bind the server to
    #[arg(short, long, default_value = "0.0.0.0:3000")]
    bind: String,

    /// Directory containing built web UI static files to serve at /
    ///
    /// When provided, the server serves these files instead of the embedded
    /// assets. Useful for development with hot-reload.
    #[arg(long)]
    web_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    info!("Starting JIT API Server...");

    // Recover before validation or command-service construction.
    let (storage, recovery_session) = prepare_server_storage(&args.data_dir)?;

    info!("Using JIT repository at: {}", args.data_dir);
    // Construct the executor over its canonical layout so any session-opening
    // command reached in-process mutates through the repository-mount boundary
    // (the worktree root is the parent of the selected data root for the D-10
    // single-mount topology). Layout construction is mandatory even for today's
    // read-only HTTP surface so future mutation paths cannot inherit a fallback.
    let data_dir_path = std::path::Path::new(&args.data_dir);
    let worktree = data_dir_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("repository data directory has no parent worktree"))?;
    let executor_layout = jit::storage::discover_repository_layout(worktree, data_dir_path)
        .context("failed to construct repository layout")?;
    let executor = Arc::new(CommandExecutor::new(storage).with_layout(executor_layout));
    if recovery_session.report().recovered_count() > 0 {
        info!(
            "Recovered {} pending transaction(s) before server startup",
            recovery_session.report().recovered_count()
        );
    }
    // The current HTTP surface is read-only. Release startup serialization
    // after validation and executor construction; any future mutation route
    // must enter through the same repository mutation boundary as the CLI.
    drop(recovery_session);

    // Start file watcher for live updates
    let (tracker, _watcher) = watcher::start_watching(&args.data_dir)?;
    let tracker = Arc::new(tracker);
    info!("Watching {} for changes", args.data_dir);

    // Derive project name from the data directory's parent (the repo root)
    let project_name = std::path::Path::new(&args.data_dir)
        .canonicalize()
        .ok()
        .and_then(|p| {
            p.parent()
                .and_then(|parent| parent.file_name().map(|n| n.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "jit".to_string());

    let state = AppState {
        executor,
        tracker,
        project_name,
    };

    // Build CORS layer for local development
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router — API always at /api, optional web UI static files at /
    let mut app = Router::new()
        .nest("/api", routes::create_routes(state))
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    // Web UI: --web-dir override > embedded assets > API-only with warning
    if let Some(web_dir) = args.web_dir.filter(|d| d.exists()) {
        info!("Serving web UI from filesystem: {}", web_dir.display());
        app = app.fallback_service(tower_http::services::ServeDir::new(web_dir));
    } else {
        #[cfg(feature = "embed-web")]
        if embedded::has_embedded_assets() {
            info!("Serving web UI from embedded assets");
            app = app.fallback(embedded::embedded_fallback);
        } else {
            warn!("Web UI not available — no embedded assets were compiled in");
            warn!("Rebuild with: cd web && npm run build && cargo build -p jit-server");
        }

        #[cfg(not(feature = "embed-web"))]
        {
            warn!("Web UI not available — built without embed-web feature");
            warn!("Rebuild with: cd web && npm run build && cargo build -p jit-server");
        }
    }

    // Start server. Prefer a socket inherited from the launching `jit`
    // process (listenfd); only bind `--bind` when none was handed down. This
    // keeps the port bound continuously across the jit→jit-server handoff.
    let (std_listener, source) = resolve_listener(&args.bind)?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    let local_addr = listener.local_addr()?;
    match source {
        ListenerSource::Inherited => info!("Server listening on http://{local_addr} (inherited)"),
        ListenerSource::Bound => info!("Server listening on http://{local_addr}"),
    }

    axum::serve(listener, app).await?;

    Ok(())
}
