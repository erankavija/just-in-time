//! JIT REST API Server
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

mod routes;
mod sse;
mod watcher;

use anyhow::Result;
use axum::Router;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use jit::commands::CommandExecutor;
use jit::storage::JsonFileStorage;
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
    /// When provided, the server serves the web UI alongside the API.
    /// If the directory does not exist, the flag is silently ignored.
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

    // Initialize storage and command executor
    let storage = JsonFileStorage::new(&args.data_dir);

    // Validate repository exists
    storage.validate().map_err(|e| {
        anyhow::anyhow!(
            "Failed to initialize storage: {}\n\n\
             The server requires a JIT repository to be initialized.\n\
             Run 'jit init' in the repository directory, or use --data-dir to point to an existing repository.",
            e
        )
    })?;

    info!("Using JIT repository at: {}", args.data_dir);
    let executor = Arc::new(CommandExecutor::new(storage));

    // Start file watcher for live updates
    let (tracker, _watcher) = watcher::start_watching(&args.data_dir)?;
    let tracker = Arc::new(tracker);
    info!("Watching {} for changes", args.data_dir);

    let state = AppState { executor, tracker };

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

    if let Some(web_dir) = args.web_dir.filter(|d| d.exists()) {
        info!("Serving web UI from {}", web_dir.display());
        app = app.fallback_service(tower_http::services::ServeDir::new(web_dir));
    }

    // Start server
    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    info!("Server listening on http://{}", args.bind);

    axum::serve(listener, app).await?;

    Ok(())
}
