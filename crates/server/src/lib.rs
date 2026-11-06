//! JIT REST API Server Library

//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

mod capture_cache;

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
use jit::commands::{open_repository, OpenedRepository, RepositoryInvocation};
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

/// Open the selected server repository with startup recovery before validation.
/// The caller releases the retained startup session before watching or serving.
///
/// # Errors
/// Returns the shared repository bootstrap's discovery, recovery, or validation error.
pub fn prepare_server_repository(data_dir: &Path, current_dir: &Path) -> Result<OpenedRepository> {
    prepare_server_repository_with_root(
        data_dir,
        current_dir,
        std::env::var_os("JIT_WORKTREE_ROOT").map(std::path::PathBuf::from),
    )
}

fn prepare_server_repository_with_root(
    data_dir: &Path,
    current_dir: &Path,
    worktree_root_override: Option<std::path::PathBuf>,
) -> Result<OpenedRepository> {
    open_repository(RepositoryInvocation {
        current_dir: current_dir.to_path_buf(),
        data_dir_override: Some(data_dir.to_path_buf()),
        worktree_root_override,
        is_init: false,
        opens_mutation_session: true,
        coordinates_claims_first: false,
        is_validation_exempt: false,
        is_recover: false,
        write_override: None,
    })
    .context("Failed to initialize server repository")
}

#[cfg(test)]
mod repository_startup_tests {
    use super::*;
    use jit::repository_state::{FileMode, VirtualPath};
    use jit::storage::{commit_redo_record, DataRoot, PutFile, RedoRecord};

    #[test]
    fn test_server_startup_passes_explicit_code_root_to_shared_bootstrap() {
        let (temp, _) = jit::test_utils::setup_test_repo().unwrap();
        let code = tempfile::tempdir().unwrap();
        let opened = prepare_server_repository_with_root(
            &temp.path().join(".jit"),
            temp.path(),
            Some(code.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(opened.layout.worktree_root(), code.path());
    }

    #[test]
    fn test_server_startup_selects_the_data_roots_checkout_from_an_outside_directory() {
        let (temp, _storage) = jit::test_utils::setup_test_repo().unwrap();
        jit::test_cli::init_git_repository(temp.path()).unwrap();
        let checkout = temp.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let opened = prepare_server_repository(&checkout.join(".jit"), outside.path()).unwrap();

        assert_eq!(opened.worktree_paths.worktree_root, checkout);
        assert!(opened.worktree_paths.is_git_repository());
        assert_eq!(opened.layout.worktree_root(), checkout);
        assert_eq!(opened.data_root, checkout.join(".jit"));
    }

    #[test]
    fn test_server_startup_preserves_the_process_directory_for_an_external_non_git_store() {
        let (temp, _storage) = jit::test_utils::setup_test_repo().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let opened = prepare_server_repository(&temp.path().join(".jit"), outside.path()).unwrap();

        assert_eq!(opened.worktree_paths.worktree_root, outside.path());
        assert!(!opened.worktree_paths.is_git_repository());
        assert_eq!(opened.layout.worktree_root(), outside.path());
        assert_eq!(opened.data_root, temp.path().join(".jit"));
    }

    #[test]
    fn test_server_startup_serves_reads_from_relative_data_dir_spellings() {
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
            let opened = prepare_server_repository(std::path::Path::new(spelling), &sibling)
                .expect("startup over a relative data directory");
            let ids: Vec<_> = opened
                .executor
                .issues()
                .list_issues(None, None, None)
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
        let record = RedoRecord::new(
            Vec::new(),
            vec![PutFile {
                path: VirtualPath::data("index.json").unwrap(),
                bytes,
                mode: FileMode::Regular,
            }],
            Vec::new(),
            None,
        )
        .unwrap();
        commit_redo_record(&DataRoot::open(&data).unwrap(), &record).unwrap();

        let opened = prepare_server_repository(&data, temp.path())
            .expect("startup applies the record before validating the restored index");

        assert_eq!(opened.recovered_records, 1);
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
