//! JIT REST API Server Library
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

#![deny(unsafe_code)]

#[cfg(feature = "embed-web")]
pub mod embedded;
pub mod routes;
pub mod shutdown;
pub mod sse;
pub mod watcher;

// Re-export for convenience
pub use routes::create_routes;

use anyhow::{Context, Result};
use jit::repository_state::RepositoryLayout;
use jit::storage::{JsonFileStorage, RepositoryMutationSession, RepositoryStateStore};
use listenfd::ListenFd;
use std::net::TcpListener;
use std::path::Path;

/// Where the server's listening socket came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerSource {
    /// Adopted from the launching process via the listenfd protocol.
    Inherited,
    /// Bound directly by this process.
    Bound,
}

/// Resolves the TCP listener the server serves on, returning it non-blocking
/// (ready for [`tokio::net::TcpListener::from_std`]) alongside its origin.
///
/// When the launching `jit` process hands down an already-bound socket through
/// the listenfd protocol (`LISTEN_FDS`), that descriptor is adopted verbatim —
/// the server performs no bind of its own, so the port `jit` probed is never
/// released and re-acquired across the process boundary (`@/inv/atomic-writes`
/// is the file analogue; this is its socket counterpart). Absent an inherited
/// fd, `bind_addr` is bound directly.
///
/// # Errors
/// Returns an error if the inherited descriptor is not a usable TCP listener,
/// or if binding `bind_addr` fails.
pub fn resolve_listener(bind_addr: &str) -> Result<(TcpListener, ListenerSource)> {
    if let Some(listener) = ListenFd::from_env()
        .take_tcp_listener(0)
        .context("Failed to adopt the listener handed down by the launching process")?
    {
        listener
            .set_nonblocking(true)
            .context("Failed to set the inherited listener non-blocking")?;
        return Ok((listener, ListenerSource::Inherited));
    }
    let listener =
        TcpListener::bind(bind_addr).with_context(|| format!("Failed to bind {bind_addr}"))?;
    listener
        .set_nonblocking(true)
        .context("Failed to set the bound listener non-blocking")?;
    Ok((listener, ListenerSource::Bound))
}

/// Recover and validate a mutation-capable server repository before constructing
/// any command executor or configuration cache.
pub fn prepare_server_storage(
    data_dir: impl AsRef<Path>,
    layout: RepositoryLayout,
) -> Result<(JsonFileStorage, Box<dyn RepositoryMutationSession>)> {
    let storage = JsonFileStorage::new(data_dir);
    let recovery_session = storage
        .open_mutation_session(layout)
        .context("Failed to recover pending repository transaction state")?;
    storage.validate().map_err(|error| {
        anyhow::anyhow!(
            "Failed to initialize storage: {}\n\n\
             The server requires a JIT repository to be initialized.\n\
             Run 'jit init' in the repository directory, or use --data-dir to point to an existing repository.",
            error
        )
    })?;
    Ok((storage, recovery_session))
}

#[cfg(test)]
mod recovery_startup_tests {
    use super::*;
    use jit::commands::CommandExecutor;
    use jit::storage::{TransactionFailureInjector, TransactionFailurePoint};
    use std::sync::Arc;
    use tempfile::TempDir;

    struct FailAfterCommit;

    impl TransactionFailureInjector for FailAfterCommit {
        fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
            if point == &TransactionFailurePoint::RepositoryAfterCommit {
                return Err(std::io::Error::other("interrupt after commit"));
            }
            Ok(())
        }
    }

    #[test]
    fn test_server_startup_recovers_committed_fresh_root_before_validation() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let layout = jit::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let interrupted =
            JsonFileStorage::with_repository_state_failures(&data, Arc::new(FailAfterCommit));
        let executor = CommandExecutor::new(interrupted).with_layout(layout);
        assert!(executor
            .initialize_fresh_repository(temp.path(), None)
            .is_err());

        let recovered_layout =
            jit::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let (_storage, session) =
            prepare_server_storage(&data, recovered_layout).expect("startup recovery");
        assert_eq!(session.recovery_report().recovered_count(), 1);
        assert!(!temp.path().join(".jit-bootstrap").exists());
    }
}

#[cfg(test)]
mod resolve_listener_tests {
    use super::*;

    /// Serializes every test that reads or writes the `LISTEN_*` environment.
    ///
    /// The process environment is global, and `ListenFd::from_env` (inside
    /// [`resolve_listener`]) CONSUMES it — listenfd removes `LISTEN_FDS` and
    /// `LISTEN_PID` after reading them — so two tests interleaving on these
    /// variables steal each other's state nondeterministically under the
    /// default parallel test runner (observed live as both spurious `Bound`
    /// and spurious `Inherited` outcomes, jit:894337e2 / jit:6eb585bc s4).
    static LISTEN_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Remove every `LISTEN_*` variable so a test starts from a clean slate
    /// even after a sibling's leftovers (listenfd does not remove
    /// `LISTEN_FDS_FIRST_FD`).
    fn scrub_listen_env() {
        std::env::remove_var("LISTEN_FDS");
        std::env::remove_var("LISTEN_FDS_FIRST_FD");
        std::env::remove_var("LISTEN_PID");
    }

    #[test]
    fn test_resolve_listener_binds_when_no_fd_inherited() {
        let _guard = LISTEN_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        scrub_listen_env();

        // No LISTEN_FDS in the environment → the server binds the address
        // itself. `:0` asks the OS for a free ephemeral port, so this test
        // never contends with a sibling over a fixed port number.
        let (listener, source) = resolve_listener("127.0.0.1:0").unwrap();
        assert_eq!(source, ListenerSource::Bound);
        assert!(listener.local_addr().unwrap().port() > 0);
    }

    /// The child-adoption path: a socket the launching process already bound
    /// is adopted verbatim, with no second bind.
    ///
    /// This is the in-process half of the cross-process handoff — the parent
    /// half (clearing close-on-exec and publishing `LISTEN_FDS*`) is covered
    /// by `commands::serve::tests::test_inherit_listener_hands_off_socket` in
    /// the `jit` crate.
    #[test]
    #[cfg(unix)]
    fn test_resolve_listener_adopts_inherited_fd() {
        use std::os::unix::io::AsRawFd;

        let _guard = LISTEN_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        scrub_listen_env();

        // Stand in for the parent: bind the real serving socket up front.
        let parent_bound = TcpListener::bind("127.0.0.1:0").unwrap();
        let bound_port = parent_bound.local_addr().unwrap().port();
        let fd = parent_bound.as_raw_fd();

        // Reproduce the environment the parent publishes for the child.
        std::env::set_var("LISTEN_FDS", "1");
        std::env::set_var("LISTEN_FDS_FIRST_FD", fd.to_string());

        let (listener, source) = resolve_listener("127.0.0.1:0").unwrap();
        assert_eq!(source, ListenerSource::Inherited);
        assert_eq!(
            listener.local_addr().unwrap().port(),
            bound_port,
            "must adopt the exact already-bound port, never re-bind a fresh one"
        );

        // `take_tcp_listener` built a new owner over the same descriptor; leak
        // the original handle so its Drop does not close the fd twice.
        std::mem::forget(parent_bound);
        // listenfd consumed LISTEN_FDS/LISTEN_PID; drop our leftover too.
        scrub_listen_env();
    }
}
