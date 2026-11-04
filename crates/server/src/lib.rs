//! JIT REST API Server Library
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

#[cfg(feature = "embed-web")]
pub mod embedded;
#[cfg(feature = "test-support")]
pub mod first_read;
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
    let inherited = ListenFd::from_env()
        .take_tcp_listener(0)
        .context("Failed to adopt the listener handed down by the launching process")?;
    listener_from(inherited, bind_addr)
}

/// Adopt `inherited` when present, else bind `bind_addr`; either listener is
/// returned non-blocking.
fn listener_from(
    inherited: Option<TcpListener>,
    bind_addr: &str,
) -> Result<(TcpListener, ListenerSource)> {
    if let Some(listener) = inherited {
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

/// Apply any leftover redo record and validate a mutation-capable server
/// repository before constructing any command executor or configuration cache.
/// Storage is rooted at the layout's normalized data root.
pub fn prepare_server_storage(
    layout: RepositoryLayout,
) -> Result<(JsonFileStorage, Box<dyn RepositoryMutationSession>)> {
    let storage = JsonFileStorage::new(layout.data_root());
    let recovery_session = storage
        .open_mutation_session(layout)
        .context("Failed to open the repository session or apply its leftover redo record")?;
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
    use jit::repository_state::{FileMode, VirtualPath};
    use jit::storage::{PutFile, RedoRecord};

    /// REQ-02, REQ-03, REQ-04: a spelling through `..` components and the
    /// normalized one bind the same layout, so reads through either serve the
    /// repository's issues. Resolving a relative `--data-dir` against the
    /// process directory is observed by the spawned server in
    /// `graceful_shutdown_tests`.
    #[test]
    fn test_server_startup_serves_reads_from_relative_data_dir_spellings() {
        use jit::storage::IssueStore;

        let (temp, _storage) = jit::test_utils::setup_test_repo().unwrap();
        let repo = temp.path().canonicalize().unwrap();
        let id = jit::test_utils::file_executor(&repo)
            .unwrap()
            .issues()
            .create_issue(
                "Hello".into(),
                String::new(),
                jit::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .expect("an issue to serve")
            .issue
            .id;

        let sibling = repo.join("sibling");
        std::fs::create_dir(&sibling).unwrap();
        for spelling in ["../.jit", "../sibling/../.jit"] {
            let layout =
                jit::storage::discover_repository_layout(&repo, sibling.join(spelling)).unwrap();
            let (storage, _session) =
                prepare_server_storage(layout).expect("startup over a relative data directory");
            let ids: Vec<_> = storage
                .list_issues()
                .expect("reads bind the layout")
                .iter()
                .map(|issue| issue.id.clone())
                .collect();
            assert!(ids.contains(&id), "{spelling} serves the repository issues");
        }
    }

    #[test]
    fn test_server_startup_applies_the_leftover_record_before_validation() {
        let (temp, _storage) = jit::test_utils::setup_test_repo().unwrap();
        let data = temp.path().join(".jit");
        let index = data.join("index.json");
        let bytes = std::fs::read(&index).unwrap();
        std::fs::remove_file(&index).unwrap();
        RedoRecord::new(
            Vec::new(),
            vec![PutFile {
                path: VirtualPath::data("index.json").unwrap(),
                bytes,
                mode: FileMode::Regular,
            }],
            Vec::new(),
            None,
        )
        .unwrap()
        .commit(&data)
        .unwrap();

        let layout = jit::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let (_storage, session) =
            prepare_server_storage(layout).expect("startup applies the record first");

        assert_eq!(session.recovery_report().recovered_count(), 1);
    }

    /// REQ-03 at the count `main` logs on startup.
    #[test]
    fn test_server_startup_counts_the_leftover_redo_record_it_applied() {
        let (temp, _storage) = jit::test_utils::setup_test_repo().unwrap();
        let data = temp.path().join(".jit");
        RedoRecord::new(
            Vec::new(),
            vec![PutFile {
                path: VirtualPath::worktree("applied-at-startup.md").unwrap(),
                bytes: b"published by the leftover record\n".to_vec(),
                mode: FileMode::Regular,
            }],
            Vec::new(),
            None,
        )
        .unwrap()
        .commit(&data)
        .unwrap();

        let served_layout = jit::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let (_storage, session) = prepare_server_storage(served_layout).expect("startup recovery");

        assert_eq!(
            session.recovery_report().recovered_count(),
            1,
            "the applied record must reach the count the server reports at startup"
        );
        assert!(temp.path().join("applied-at-startup.md").exists());
    }
}

#[cfg(test)]
mod resolve_listener_tests {
    use super::*;

    #[test]
    fn test_listener_from_binds_when_no_listener_inherited() {
        // `:0` asks the OS for a free ephemeral port, so this test never
        // contends with a sibling over a fixed port number.
        let (listener, source) = listener_from(None, "127.0.0.1:0").unwrap();
        assert_eq!(source, ListenerSource::Bound);
        assert!(listener.local_addr().unwrap().port() > 0);
    }

    /// The child-adoption path: a socket the launching process already bound
    /// is adopted verbatim, with no second bind.
    ///
    /// The parent half (clearing close-on-exec and publishing `LISTEN_FDS*`)
    /// is covered by `commands::serve::tests::test_inherit_listener_hands_off_socket`
    /// in the `jit` crate, and the whole handoff by the `serve_cli_tests` that
    /// start a server through `jit serve`.
    #[test]
    fn test_listener_from_adopts_inherited_listener() {
        let parent_bound = TcpListener::bind("127.0.0.1:0").unwrap();
        let bound_port = parent_bound.local_addr().unwrap().port();

        let (listener, source) = listener_from(Some(parent_bound), "127.0.0.1:0").unwrap();
        assert_eq!(source, ListenerSource::Inherited);
        assert_eq!(
            listener.local_addr().unwrap().port(),
            bound_port,
            "must adopt the exact already-bound port, never re-bind a fresh one"
        );
    }
}
