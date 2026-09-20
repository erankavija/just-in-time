//! Applying a redo record to the repository's filesystem roots.
//!
//! One forward pass over the record's operations, then the record is removed.
//! Re-application is the same pass: a put overwrites, a delete tolerates
//! absence, the append rewrites its bytes at its offset, so a record applied
//! twice converges. Nothing publishes through this module yet.

use crate::repository_state::VirtualPath;
use crate::storage::redo_record::{EventAppend, PutFile, RedoRecord};
use crate::storage::repository_state_store::{
    open_capability_parent, CapabilityRoots, RepositoryStateStoreError,
};
use cap_std::fs::Dir;
use std::path::Path;

/// One step of a record's application, named so an interruption matrix can stop
/// the pass at it.
///
/// A point names the state before its step runs, and both backends step these
/// in this order, so an injected interruption means the same thing on either.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RedoPoint {
    /// Every path resolved, before the first live write.
    BeforeApply,
    /// Before put `index` replaces its path.
    BeforePut { index: usize },
    /// Before delete `index` removes its path.
    BeforeDelete { index: usize },
    /// Before the appended bytes are written at their offset.
    BeforeEventAppend,
    /// Every operation applied, before the record is removed.
    BeforeRecordRemoval,
    /// The record is gone: the mutation is complete.
    AfterApply,
}

/// Test seam for interrupting one named step of a record's application.
pub trait RedoInterruptionInjector: Send + Sync {
    /// Interrupt the pass at one point, or let it proceed.
    fn check(&self, point: &RedoPoint) -> std::io::Result<()>;
}

/// Production injector: every point proceeds.
#[derive(Debug, Default)]
pub struct NoRedoInterruptions;

impl RedoInterruptionInjector for NoRedoInterruptions {
    fn check(&self, _point: &RedoPoint) -> std::io::Result<()> {
        Ok(())
    }
}

/// Failures of one record application.
#[derive(Debug, thiserror::Error)]
pub enum RedoApplyError {
    #[error("redo record addresses {path}, which the selected roots do not hold")]
    Unaddressable { path: String },
    #[error("redo application interrupted at {point:?}: {source}")]
    Interrupted {
        point: RedoPoint,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Roots(#[from] RepositoryStateStoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Apply `record` to `roots` and remove it, checking every [`RedoPoint`].
///
/// Every path is resolved before the first live write, so a record naming a
/// target outside the selected roots changes nothing.
///
/// # Errors
///
/// Returns [`RedoApplyError`] when a path is unaddressable, an injected point
/// interrupts the pass, or the filesystem refuses an operation.
pub fn apply_record(
    roots: &CapabilityRoots,
    record: &RedoRecord,
    injector: &dyn RedoInterruptionInjector,
) -> Result<(), RedoApplyError> {
    let event_log = VirtualPath::EVENTS;
    record
        .puts
        .iter()
        .map(|put| &put.path)
        .chain(record.deletes.iter())
        .chain(record.event_append.iter().map(|_| &event_log))
        .try_for_each(|path| resolve(roots, path).map(|_| ()))?;
    check(injector, RedoPoint::BeforeApply)?;
    record
        .puts
        .iter()
        .enumerate()
        .try_for_each(|(index, put)| {
            check(injector, RedoPoint::BeforePut { index })?;
            put_file(roots, put)
        })?;
    record
        .deletes
        .iter()
        .enumerate()
        .try_for_each(|(index, path)| {
            check(injector, RedoPoint::BeforeDelete { index })?;
            delete_file(roots, path)
        })?;
    if let Some(append) = &record.event_append {
        check(injector, RedoPoint::BeforeEventAppend)?;
        append_events(roots, append)?;
    }
    check(injector, RedoPoint::BeforeRecordRemoval)?;
    remove_record(roots)?;
    check(injector, RedoPoint::AfterApply)
}

fn check(
    injector: &dyn RedoInterruptionInjector,
    point: RedoPoint,
) -> Result<(), RedoApplyError> {
    injector
        .check(&point)
        .map_err(|source| RedoApplyError::Interrupted { point, source })
}

/// The root handle a path is addressed within, with its root-relative portion.
fn resolve<'a>(
    roots: &'a CapabilityRoots,
    path: &'a VirtualPath,
) -> Result<(&'a Dir, &'a Path), RedoApplyError> {
    let root = roots
        .root(path.root_class())
        .ok_or_else(|| unaddressable(path))?;
    open_capability_parent(root, path.relative().as_path())?;
    Ok((root, path.relative().as_path()))
}

fn put_file(_roots: &CapabilityRoots, _put: &PutFile) -> Result<(), RedoApplyError> {
    Ok(())
}

fn delete_file(_roots: &CapabilityRoots, _path: &VirtualPath) -> Result<(), RedoApplyError> {
    Ok(())
}

fn append_events(_roots: &CapabilityRoots, _append: &EventAppend) -> Result<(), RedoApplyError> {
    Ok(())
}

fn remove_record(_roots: &CapabilityRoots) -> Result<(), RedoApplyError> {
    Ok(())
}

fn unaddressable(path: &VirtualPath) -> RedoApplyError {
    RedoApplyError::Unaddressable {
        path: format!("{path:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{FileMode, RepositoryRootClass};
    use crate::storage::discover_repository_layout;
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// The event-log bytes a mutation captured before appending to them.
    const CAPTURED_LOG: &[u8] = b"{\"tag\":\"issue_created\"}\n";

    /// A repository whose worktree holds its data root, with capability handles
    /// for both.
    fn repository() -> (TempDir, CapabilityRoots) {
        let worktree = TempDir::new().unwrap();
        std::fs::create_dir(worktree.path().join(".jit")).unwrap();
        let layout =
            discover_repository_layout(worktree.path(), worktree.path().join(".jit")).unwrap();
        let roots = CapabilityRoots::open(&layout).unwrap().0;
        (worktree, roots)
    }

    fn data_root(worktree: &TempDir) -> PathBuf {
        worktree.path().join(".jit")
    }

    fn put(path: VirtualPath, bytes: &[u8]) -> PutFile {
        PutFile {
            path,
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
    }

    /// Records every point the pass checks, in order.
    #[derive(Default)]
    struct RecordedPoints(Mutex<Vec<RedoPoint>>);

    impl RecordedPoints {
        fn points(&self) -> Vec<RedoPoint> {
            self.0.lock().unwrap().clone()
        }
    }

    impl RedoInterruptionInjector for RecordedPoints {
        fn check(&self, point: &RedoPoint) -> std::io::Result<()> {
            self.0.lock().unwrap().push(point.clone());
            Ok(())
        }
    }

    /// Interrupts the pass at its `at`-th check, so exactly the steps before it
    /// ran.
    struct InterruptAt {
        at: usize,
        seen: AtomicUsize,
    }

    impl InterruptAt {
        fn check_number(at: usize) -> Self {
            Self {
                at,
                seen: AtomicUsize::new(0),
            }
        }
    }

    impl RedoInterruptionInjector for InterruptAt {
        fn check(&self, point: &RedoPoint) -> std::io::Result<()> {
            match self.seen.fetch_add(1, Ordering::Relaxed) == self.at {
                true => Err(std::io::Error::other(format!("interrupted at {point:?}"))),
                false => Ok(()),
            }
        }
    }

    /// Every file under `root`, keyed by its root-relative path, with the bytes
    /// and the executable bit that decide whether two repositories converged.
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, bool)> {
        std::fs::read_dir(root)
            .unwrap()
            .flat_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                match entry.file_type().unwrap().is_dir() {
                    true => snapshot(&path)
                        .into_iter()
                        .map(|(nested, content)| {
                            (PathBuf::from(entry.file_name()).join(nested), content)
                        })
                        .collect::<BTreeMap<_, _>>(),
                    false => BTreeMap::from([(
                        PathBuf::from(entry.file_name()),
                        (std::fs::read(&path).unwrap(), is_executable(&path)),
                    )]),
                }
            })
            .collect()
    }

    #[cfg(unix)]
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    fn is_executable(_path: &Path) -> bool {
        false
    }

    #[test]
    fn test_apply_record_publishes_every_operation_the_record_names() {
        let (worktree, roots) = repository();
        std::fs::write(data_root(&worktree).join("index.json"), b"captured").unwrap();
        std::fs::write(worktree.path().join("stale.md"), b"stale").unwrap();
        std::fs::write(data_root(&worktree).join("events.jsonl"), CAPTURED_LOG).unwrap();
        let record = RedoRecord {
            puts: vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            deletes: vec![VirtualPath::worktree("stale.md").unwrap()],
            event_append: Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        };

        apply_record(&roots, &record, &NoRedoInterruptions).unwrap();

        assert_eq!(
            std::fs::read(data_root(&worktree).join("index.json")).unwrap(),
            b"published"
        );
        assert!(
            !worktree.path().join("stale.md").exists(),
            "the deleted path survived the application"
        );
        assert_eq!(
            std::fs::read(data_root(&worktree).join("events.jsonl")).unwrap(),
            [CAPTURED_LOG, b"{\"tag\":\"issue_claimed\"}\n"].concat()
        );
    }

    #[test]
    fn test_apply_record_removes_the_record_it_applied() {
        let (worktree, roots) = repository();
        let record = RedoRecord {
            puts: vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            ..RedoRecord::default()
        };
        let committed = record.commit(&data_root(&worktree)).unwrap();

        apply_record(&roots, &record, &NoRedoInterruptions).unwrap();

        assert!(
            !committed.exists(),
            "the applied record is still there for a later session to apply again"
        );
    }

    #[test]
    fn test_apply_record_creates_the_directories_a_put_needs() {
        let (worktree, roots) = repository();
        let record = RedoRecord {
            puts: vec![put(
                VirtualPath::data("issues/2026/a.json").unwrap(),
                b"published",
            )],
            ..RedoRecord::default()
        };

        apply_record(&roots, &record, &NoRedoInterruptions).unwrap();

        assert_eq!(
            std::fs::read(data_root(&worktree).join("issues/2026/a.json")).unwrap(),
            b"published"
        );
    }

    #[test]
    fn test_apply_record_gives_each_put_the_mode_it_carries() {
        let (worktree, roots) = repository();
        let record = RedoRecord {
            puts: vec![PutFile {
                path: VirtualPath::worktree("scripts/publish.sh").unwrap(),
                bytes: b"#!/bin/sh\n".to_vec(),
                mode: FileMode::Executable,
            }],
            ..RedoRecord::default()
        };

        apply_record(&roots, &record, &NoRedoInterruptions).unwrap();

        assert!(
            is_executable(&worktree.path().join("scripts/publish.sh")),
            "the published file does not carry the mode the record named"
        );
    }

    #[test]
    fn test_apply_record_writes_the_append_at_its_offset_over_a_longer_log() {
        let (worktree, roots) = repository();
        let appended = b"{\"tag\":\"issue_claimed\"}\n";
        // A log an interrupted application already extended: longer than the
        // offset, so append mode would land the bytes past the residue and a
        // pass that never truncates would leave it behind.
        std::fs::write(
            data_root(&worktree).join("events.jsonl"),
            [CAPTURED_LOG, b"{\"tag\":\"residue\"}\n"].concat(),
        )
        .unwrap();
        let record = RedoRecord {
            event_append: Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: appended.to_vec(),
            }),
            ..RedoRecord::default()
        };

        apply_record(&roots, &record, &NoRedoInterruptions).unwrap();

        assert_eq!(
            std::fs::read(data_root(&worktree).join("events.jsonl")).unwrap(),
            [CAPTURED_LOG, appended].concat()
        );
    }

    #[test]
    fn test_apply_record_checks_every_interruption_point_in_order() {
        let (worktree, roots) = repository();
        std::fs::write(data_root(&worktree).join("events.jsonl"), CAPTURED_LOG).unwrap();
        let record = RedoRecord {
            puts: vec![
                put(VirtualPath::data("index.json").unwrap(), b"published"),
                put(VirtualPath::worktree("notes.md").unwrap(), b"published"),
            ],
            deletes: vec![VirtualPath::worktree("stale.md").unwrap()],
            event_append: Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        };
        let recorded = RecordedPoints::default();

        apply_record(&roots, &record, &recorded).unwrap();

        assert_eq!(
            recorded.points(),
            vec![
                RedoPoint::BeforeApply,
                RedoPoint::BeforePut { index: 0 },
                RedoPoint::BeforePut { index: 1 },
                RedoPoint::BeforeDelete { index: 0 },
                RedoPoint::BeforeEventAppend,
                RedoPoint::BeforeRecordRemoval,
                RedoPoint::AfterApply,
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_apply_record_refuses_a_path_outside_the_roots_before_any_live_write() {
        let (worktree, roots) = repository();
        let outside = TempDir::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), data_root(&worktree).join("escape")).unwrap();
        let record = RedoRecord {
            puts: vec![
                put(VirtualPath::data("inside.json").unwrap(), b"published"),
                put(
                    VirtualPath::data("escape/captured.json").unwrap(),
                    b"published",
                ),
            ],
            ..RedoRecord::default()
        };

        assert!(apply_record(&roots, &record, &NoRedoInterruptions).is_err());

        assert!(
            !data_root(&worktree).join("inside.json").exists(),
            "a live write ran before the record was refused"
        );
        assert!(
            !outside.path().join("captured.json").exists(),
            "the application wrote outside the selected roots"
        );
    }

    /// Paths a generated record and its pre-existing state draw from: two roots,
    /// nested and unnested, so puts and deletes collide on the same targets.
    fn path_vocabulary() -> impl Strategy<Value = VirtualPath> {
        prop_oneof![
            Just(VirtualPath::data("index.json").unwrap()),
            Just(VirtualPath::data("issues/2026/a.json").unwrap()),
            Just(VirtualPath::worktree("notes.md").unwrap()),
            Just(VirtualPath::worktree("dev/plan.md").unwrap()),
        ]
    }

    fn arbitrary_record() -> impl Strategy<Value = RedoRecord> {
        (
            prop::collection::vec(
                (
                    path_vocabulary(),
                    prop::collection::vec(any::<u8>(), 0..8),
                    prop_oneof![Just(FileMode::Regular), Just(FileMode::Executable)],
                ),
                0..4,
            ),
            prop::collection::vec(path_vocabulary(), 0..3),
            prop::option::of(prop::collection::vec(any::<u8>(), 0..8)),
        )
            .prop_map(|(puts, deletes, appended)| RedoRecord {
                puts: puts
                    .into_iter()
                    .map(|(path, bytes, mode)| PutFile { path, bytes, mode })
                    .collect(),
                deletes,
                event_append: appended.map(|bytes| EventAppend {
                    offset: CAPTURED_LOG.len() as u64,
                    bytes,
                }),
            })
    }

    /// Pre-existing targets, and whether the log carries residue beyond the
    /// offset the record captured.
    fn arbitrary_initial_state() -> impl Strategy<Value = (Vec<VirtualPath>, bool)> {
        (
            prop::collection::vec(path_vocabulary(), 0..3),
            any::<bool>(),
        )
    }

    fn seed(worktree: &TempDir, (occupied, residue): &(Vec<VirtualPath>, bool)) {
        let log = match residue {
            true => [CAPTURED_LOG, b"{\"tag\":\"residue\"}\n"].concat(),
            false => CAPTURED_LOG.to_vec(),
        };
        std::fs::write(data_root(worktree).join("events.jsonl"), log).unwrap();
        occupied.iter().for_each(|path| {
            let target = match path.root_class() {
                RepositoryRootClass::Data => data_root(worktree),
                RepositoryRootClass::Worktree => worktree.path().to_path_buf(),
            }
            .join(path.relative().as_path());
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, b"captured").unwrap();
        });
    }

    /// A repository seeded with `initial`, holding `record` as its committed
    /// mutation.
    fn committed(record: &RedoRecord, initial: &(Vec<VirtualPath>, bool)) -> (TempDir, CapabilityRoots) {
        let (worktree, roots) = repository();
        seed(&worktree, initial);
        record.commit(&data_root(&worktree)).unwrap();
        (worktree, roots)
    }

    proptest! {
        // Each case applies the record once per prefix of its operation order,
        // each application over its own repository, so the case count stays low.
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// The record's own point sequence enumerates every prefix, so an
        /// interruption at each one, resumed, is compared against a single clean
        /// application of the same record over the same initial state.
        #[test]
        fn prop_re_application_after_any_interrupted_prefix_converges(
            record in arbitrary_record(),
            initial in arbitrary_initial_state(),
        ) {
            let (clean, clean_roots) = committed(&record, &initial);
            let recorded = RecordedPoints::default();
            apply_record(&clean_roots, &record, &recorded).unwrap();
            let converged = snapshot(clean.path());

            for prefix in 0..recorded.points().len() {
                let (interrupted, interrupted_roots) = committed(&record, &initial);
                let outcome = apply_record(
                    &interrupted_roots,
                    &record,
                    &InterruptAt::check_number(prefix),
                );
                prop_assert!(
                    matches!(outcome, Err(RedoApplyError::Interrupted { .. })),
                    "the pass ran past the interruption at check {prefix}"
                );

                apply_record(&interrupted_roots, &record, &NoRedoInterruptions).unwrap();

                prop_assert_eq!(
                    snapshot(interrupted.path()),
                    converged.clone(),
                    "re-application after check {} left a different repository",
                    prefix
                );
            }
        }
    }
}
