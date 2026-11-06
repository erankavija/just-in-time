//! The read driver over both backends: admission, bounded conflicts, and recovery isolation.

use super::with_read_session;
use super::{capture_discovering, READ_OPTIMISTIC_ATTEMPTS};
use crate::repository_state::{CaptureBudget, CaptureSpec};
use crate::storage::repository_state_store::test_fixtures::{
    leftover_record, leftover_target, FileSessionBackend, MemorySessionBackend,
    SessionRecordBackend, LEFTOVER_BYTES,
};
use crate::storage::RepositoryReadSession;
use anyhow::{anyhow, Result};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

fn leftover_spec() -> CaptureSpec {
    CaptureSpec::new(
        [leftover_target()],
        CaptureBudget {
            max_listings: 0,
            max_bytes: 1024 * 1024,
            max_depth: 8,
        },
    )
    .unwrap()
}

/// The bytes at [`leftover_target`], read through `session`.
fn read_leftover(session: &mut dyn RepositoryReadSession) -> Result<Option<Option<Vec<u8>>>> {
    let image = session.capture(leftover_spec())?;
    Ok(Some(
        image.file_bytes(&leftover_target())?.map(<[u8]>::to_vec),
    ))
}

/// A barrier whose wait gives up after a timeout instead of hanging.
struct TimedBarrier {
    arrived: Mutex<usize>,
    all: Condvar,
    parties: usize,
}

impl TimedBarrier {
    fn new(parties: usize) -> Self {
        Self {
            arrived: Mutex::new(0),
            all: Condvar::new(),
            parties,
        }
    }

    /// Whether every party arrived within `timeout`.
    fn wait(&self, timeout: Duration) -> bool {
        let mut arrived = self.arrived.lock().unwrap();
        *arrived += 1;
        self.all.notify_all();
        let (_arrived, waited) = self
            .all
            .wait_timeout_while(arrived, timeout, |arrived| *arrived < self.parties)
            .unwrap();
        !waited.timed_out()
    }
}

#[test]
fn test_capture_discovering_through_the_read_trait_object_yields_the_mutation_image() {
    let backend = MemorySessionBackend::seeded();
    backend.commit(&leftover_record());
    let mut session = backend.open().unwrap();

    let through_mutation = session.capture(leftover_spec()).unwrap();
    let read: &mut dyn RepositoryReadSession = session.as_mut();
    let through_read = capture_discovering(read, leftover_spec(), |_, _| Ok(()))
        .unwrap()
        .expect("nothing conflicts inside a held session");

    assert_eq!(through_read.entries(), through_mutation.entries());
    assert_eq!(through_read.capture_spec(), through_mutation.capture_spec());
}

#[test]
fn test_with_read_session_returns_an_admitted_error() {
    let backend = MemorySessionBackend::seeded();
    let mut calls = 0;

    let outcome: Result<()> =
        with_read_session(backend.store(), &backend.layout(), "admitted", |_| {
            calls += 1;
            Err(anyhow!("derivation failed"))
        });

    assert_eq!(outcome.unwrap_err().to_string(), "derivation failed");
    assert_eq!(calls, 1, "an admitted error ends the read");
}

#[test]
fn test_with_read_session_discards_the_error_of_an_attempt_a_commit_crossed() {
    let backend = MemorySessionBackend::seeded();
    let mut calls = 0;

    let value = with_read_session(backend.store(), &backend.layout(), "crossed", |session| {
        calls += 1;
        if calls == 1 {
            // A complete commit inside the capture: the open applies it.
            backend.commit(&leftover_record());
            drop(backend.open()?);
            return Err(anyhow!("derived from a torn image"));
        }
        read_leftover(session)
    })
    .unwrap();

    assert_eq!(value.as_deref(), Some(LEFTOVER_BYTES));
    assert_eq!(calls, 2, "the crossed attempt is retried");
}

#[test]
fn test_with_read_session_returns_conflict_after_bounded_attempts() {
    let backend = MemorySessionBackend::seeded();
    let mut calls = 0;
    let outcome: Result<()> =
        with_read_session(backend.store(), &backend.layout(), "unsettled", |_| {
            calls += 1;
            Ok(None)
        });
    assert!(matches!(
        outcome
            .unwrap_err()
            .downcast_ref::<crate::storage::RepositoryStateStoreError>(),
        Some(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
    ));
    assert_eq!(calls, READ_OPTIMISTIC_ATTEMPTS as usize);
}

#[test]
fn test_with_read_session_refuses_contended_memory_state_without_waiting() {
    let backend = MemorySessionBackend::seeded();
    let store = backend.0.clone();
    let layout = backend.layout();
    let held = backend.0.repository_state();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result: Result<()> = with_read_session(&store, &layout, "contended", |_| Ok(Some(())));
        tx.send(result).unwrap();
    });
    let result = rx.recv_timeout(Duration::from_secs(2));
    drop(held);
    thread.join().unwrap();
    let error = result.unwrap().unwrap_err();
    assert!(matches!(
        error.downcast_ref::<crate::storage::RepositoryStateStoreError>(),
        Some(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
    ));
}

#[test]
fn test_resolve_issue_id_propagates_contended_memory_conflict() {
    let store = crate::storage::InMemoryStorage::new();
    let executor = super::test_helpers::memory_executor(store.clone());
    let held = store.repository_state();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        tx.send(executor.issues().resolve_issue_id(&"0000".parse().unwrap()))
            .unwrap();
    });
    let result = rx.recv_timeout(Duration::from_secs(2));
    drop(held);
    thread.join().unwrap();
    let error = result.unwrap().unwrap_err();
    assert!(matches!(
        error.downcast_ref::<crate::storage::RepositoryStateStoreError>(),
        Some(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
    ));
}

/// Every reader holds its read session open at the barrier, so all of them
/// pass it only if none waits on the held mutation session or on another
/// reader.
fn assert_reads_proceed_beside_a_held_mutation_session<B: SessionRecordBackend>() {
    const READERS: usize = 8;
    let backend = B::seeded();
    let barrier = TimedBarrier::new(READERS);
    let held = backend.open().unwrap();

    let passed = std::thread::scope(|scope| {
        let readers = (0..READERS)
            .map(|_| {
                let (store, layout, barrier) =
                    (backend.store().clone(), backend.layout(), &barrier);
                scope.spawn(move || {
                    with_read_session(&store, &layout, "beside a writer", |session| {
                        session.capture(leftover_spec())?;
                        Ok(Some(barrier.wait(Duration::from_secs(10))))
                    })
                })
            })
            .collect::<Vec<_>>();
        readers
            .into_iter()
            .map(|reader| reader.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(passed, vec![true; READERS]);
    drop(held);
}

#[test]
fn test_with_read_session_reads_beside_a_held_mutation_session_on_both_backends() {
    assert_reads_proceed_beside_a_held_mutation_session::<FileSessionBackend>();
    assert_reads_proceed_beside_a_held_mutation_session::<MemorySessionBackend>();
}

fn assert_a_read_leaves_a_leftover_record<B: SessionRecordBackend>() {
    let backend = B::seeded();
    backend.commit(&leftover_record());
    let error = with_read_session(
        backend.store(),
        &backend.layout(),
        "leftover",
        read_leftover,
    )
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<crate::storage::RepositoryStateStoreError>(),
        Some(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
    ));
    assert!(backend.holds_record(), "a read must not apply the record");
    drop(backend.open().unwrap());
    assert!(
        !backend.holds_record(),
        "an explicit mutation session applies it"
    );
}

#[test]
fn test_with_read_session_leaves_a_leftover_record_on_both_backends() {
    assert_a_read_leaves_a_leftover_record::<FileSessionBackend>();
    assert_a_read_leaves_a_leftover_record::<MemorySessionBackend>();
}

#[test]
fn test_captured_declarations_matches_the_parse_of_the_captured_image_and_writes_nothing() {
    use crate::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
    use crate::repository_state::VirtualPath;
    use crate::storage::RepositoryStateStore;

    let schema = VirtualPath::schema("captured.json").unwrap();
    let rules = format!(
        "[[rules]]\nname = \"captured-schema\"\nseverity = \"error\"\nassert = {{ json-schema = {:?} }}\n",
        schema.relative().as_str(),
    );
    let storage = crate::storage::InMemoryStorage::new();
    crate::test_harness::seed_memory_repository_file(
        &storage,
        &schema.repository_relative(),
        r#"{ "type": "object" }"#,
    );
    let config = crate::test_taxonomy::test_taxonomy().config_fragment();
    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(&config),
            rules: Some(&rules),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let layout = executor.require_layout().unwrap();
    let image = executor
        .storage()
        .open_read_session(layout)
        .unwrap()
        .capture(
            CaptureSpec::new(
                VirtualPath::DECLARATION_FILES.into_iter().chain([schema]),
                CaptureBudget {
                    max_listings: 0,
                    max_bytes: 1024 * 1024,
                    max_depth: 8,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let parsed = crate::repository_state::declarations_from_image(&image).unwrap();
    let before = executor.storage().repository_state().entries.clone();

    let captured = executor.config().captured_declarations().unwrap();

    assert_eq!(executor.storage().repository_state().entries, before);
    let snapshot = |declarations: &crate::repository_state::CapturedRepositoryDeclarations| {
        crate::config::EffectiveConfig::new(declarations.config().clone())
            .full_snapshot()
            .unwrap()
    };
    assert_eq!(snapshot(&captured), snapshot(&parsed));
    assert_eq!(captured.rules(), parsed.rules());
    assert!(captured
        .rules()
        .rules
        .iter()
        .any(|rule| rule.name == "captured-schema"));
    assert_eq!(
        serde_json::to_value(captured.namespaces()).unwrap(),
        serde_json::to_value(parsed.namespaces()).unwrap()
    );
}
