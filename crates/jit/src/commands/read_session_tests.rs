//! The read driver over both backends: what it surfaces, when it falls back to
//! the mutation session, and that its attempts never wait on a writer.

use super::{capture_discovering, READ_OPTIMISTIC_ATTEMPTS};
use super::{with_read_session, MutationSessionExhausted, MUTATION_SESSION_RETRY_LIMIT};
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
fn test_with_read_session_falls_back_to_one_mutation_session_drive_after_failed_attempts() {
    let backend = MemorySessionBackend::seeded();
    let mut calls = 0;

    let outcome: Result<()> =
        with_read_session(backend.store(), &backend.layout(), "unsettled", |_| {
            calls += 1;
            Ok(None)
        });

    assert!(outcome
        .unwrap_err()
        .downcast_ref::<MutationSessionExhausted>()
        .is_some());
    assert_eq!(
        calls,
        READ_OPTIMISTIC_ATTEMPTS as usize + MUTATION_SESSION_RETRY_LIMIT,
        "each optimistic attempt runs once, then the mutation session's own bound"
    );
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

fn assert_a_read_completes_a_leftover_record<B: SessionRecordBackend>() {
    let backend = B::seeded();
    backend.commit(&leftover_record());

    let read = with_read_session(
        backend.store(),
        &backend.layout(),
        "leftover",
        read_leftover,
    )
    .unwrap();

    assert_eq!(read.as_deref(), Some(LEFTOVER_BYTES));
    assert!(!backend.holds_record(), "the locked attempt applied it");
}

#[test]
fn test_with_read_session_completes_a_leftover_record_on_both_backends() {
    assert_a_read_completes_a_leftover_record::<FileSessionBackend>();
    assert_a_read_completes_a_leftover_record::<MemorySessionBackend>();
}
