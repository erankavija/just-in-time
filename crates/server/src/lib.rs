//! JIT REST API Server Library
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

#[cfg(feature = "embed-web")]
pub mod embedded;
pub mod routes;
pub mod sse;
pub mod watcher;

// Re-export for convenience
pub use routes::create_routes;
