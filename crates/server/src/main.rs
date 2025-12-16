//! JIT REST API Server
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

mod routes;

use anyhow::Result;
use axum::Router;
use clap::Parser;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use jit::commands::CommandExecutor;
use jit::storage::JsonFileStorage;

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

    // Build CORS layer for local development
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router
    let app = Router::new()
        .nest("/api", routes::create_routes(executor))
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    // Start server
    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    info!("Server listening on http://{}", args.bind);

    axum::serve(listener, app).await?;

    Ok(())
}
