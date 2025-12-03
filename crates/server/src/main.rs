//! JIT REST API Server
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

mod routes;

use anyhow::Result;
use axum::Router;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use jit::commands::CommandExecutor;
use jit::storage::JsonFileStorage;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    info!("Starting JIT API Server...");

    // Initialize storage and command executor
    let storage = JsonFileStorage::new(String::from("."));
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
    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
