//! Integration suite for the `jit-server` crate.
//!
//! One Cargo target for the whole crate's integration layer, so the workspace
//! stays inside the enforced integration-target budget
//! (`@/inv/bounded-rust-build-footprint`). Add a case to the module that
//! matches its subsystem, or add a module file plus a `mod` line here.
//!
//! * `document_api_tests` — the document endpoints, in process over
//!   `axum_test::TestServer`.
//! * `graceful_shutdown_tests` — the compiled `jit-server` binary as a real
//!   process, signalled and observed from the outside (Unix only: the scenario
//!   is defined by SIGTERM delivery).

mod document_api_tests;

#[cfg(unix)]
mod graceful_shutdown_tests;
