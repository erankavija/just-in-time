//! JIT REST API Server Library
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

pub mod routes;

// Re-export for convenience
pub use routes::create_routes;
