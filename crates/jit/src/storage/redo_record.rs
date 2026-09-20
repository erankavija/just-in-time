//! The redo record: one mutation's file changes, described once and applied forward.
//!
//! A record carries the complete new bytes of every file the mutation publishes,
//! the paths it removes, and the bytes it appends to the event log at a captured
//! offset. Re-applying a record converges on the same repository state, so a
//! record that survives a crash is the mutation. Parent directories are implied:
//! a put creates the directories its path needs.

use crate::repository_state::{
    FileMode, RepositoryLayoutError, RepositoryRootClass, RootRelativePath, VirtualPath,
};
use serde::{Deserialize, Serialize};

/// Scratch directory below the data root that holds the record.
pub const RECORD_SCRATCH_DIR: &str = "tmp";

/// The record's file name below [`RECORD_SCRATCH_DIR`].
///
/// The extension is deliberately not `.tmp`: the orphan sweep in
/// [`super::temp_cleanup`] matches that extension and recurses into
/// subdirectories, so a `.tmp` record could be removed out from under a pending
/// apply.
pub const RECORD_FILE_NAME: &str = "redo.record";

const FORMAT_VERSION: u64 = 1;

/// A file published whole: its complete new bytes and the mode it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutFile {
    pub path: VirtualPath,
    pub bytes: Vec<u8>,
    pub mode: FileMode,
}

/// Event-log bytes written at the offset the mutation captured as the log's length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAppend {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// One mutation's file changes: ordered puts, ordered deletes, one event append.
///
/// A move is a put and a delete in the same record. Paths are canonical
/// identities, so one record addresses data-root and worktree targets alike and
/// nothing outside them; parsing re-applies that guard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedoRecord {
    pub puts: Vec<PutFile>,
    pub deletes: Vec<VirtualPath>,
    pub event_append: Option<EventAppend>,
}

impl RedoRecord {
    /// The record's canonical serialized form: one JSON header line naming every
    /// operation and the length of the bytes it claims, then those bytes in the
    /// order the header names them.
    ///
    /// The header is compact JSON, so it holds no raw newline and the first
    /// newline ends it. Content stays raw rather than encoded, because a put
    /// carries whole files — the non-UTF-8 assets a profile publishes, and,
    /// while the event log is still a put, the whole log. Equal records
    /// serialize to equal bytes and nothing process-local is written, so the
    /// record one process wrote is the record another parses.
    ///
    /// # Errors
    ///
    /// Returns [`RedoRecordError::Header`] when the header cannot be encoded.
    pub fn serialize(&self) -> Result<Vec<u8>, RedoRecordError> {
        let header = RecordHeader {
            version: FORMAT_VERSION,
            puts: self
                .puts
                .iter()
                .map(|put| PutHeader {
                    root: put.path.root_class(),
                    relative: relative_spelling(&put.path),
                    mode: put.mode,
                    length: put.bytes.len() as u64,
                })
                .collect(),
            deletes: self
                .deletes
                .iter()
                .map(|path| PathHeader {
                    root: path.root_class(),
                    relative: relative_spelling(path),
                })
                .collect(),
            event_append: self.event_append.as_ref().map(|append| AppendHeader {
                offset: append.offset,
                length: append.bytes.len() as u64,
            }),
        };
        let mut bytes = serde_json::to_vec(&header)
            .map_err(|source| RedoRecordError::Header(source.to_string()))?;
        bytes.push(b'\n');
        self.puts
            .iter()
            .for_each(|put| bytes.extend_from_slice(&put.bytes));
        if let Some(append) = &self.event_append {
            bytes.extend_from_slice(&append.bytes);
        }
        Ok(bytes)
    }

    /// Parse a record from its serialized form.
    ///
    /// # Errors
    ///
    /// Returns [`RedoRecordError`] when the bytes carry no header, a header this
    /// build cannot read, a path outside the repository roots, fewer content
    /// bytes than the header declares, or bytes beyond them.
    pub fn parse(bytes: &[u8]) -> Result<Self, RedoRecordError> {
        let end = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or(RedoRecordError::MissingHeader)?;
        let (line, rest) = bytes.split_at(end);
        let header: RecordHeader = serde_json::from_slice(line)
            .map_err(|source| RedoRecordError::Header(source.to_string()))?;
        if header.version != FORMAT_VERSION {
            return Err(RedoRecordError::UnsupportedVersion(header.version));
        }
        let content = rest.get(1..).unwrap_or_default();
        let mut read = 0;
        let puts = header
            .puts
            .into_iter()
            .map(|put| {
                Ok(PutFile {
                    path: canonical_path(put.root, put.relative)?,
                    bytes: take_content(content, &mut read, put.length)?.to_vec(),
                    mode: put.mode,
                })
            })
            .collect::<Result<Vec<_>, RedoRecordError>>()?;
        let deletes = header
            .deletes
            .into_iter()
            .map(|path| canonical_path(path.root, path.relative))
            .collect::<Result<Vec<_>, RedoRecordError>>()?;
        let event_append = header
            .event_append
            .map(|append| -> Result<EventAppend, RedoRecordError> {
                Ok(EventAppend {
                    offset: append.offset,
                    bytes: take_content(content, &mut read, append.length)?.to_vec(),
                })
            })
            .transpose()?;
        match content.len() - read {
            0 => Ok(Self {
                puts,
                deletes,
                event_append,
            }),
            trailing => Err(RedoRecordError::TrailingBytes(trailing)),
        }
    }
}

/// A serialized record this build cannot read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RedoRecordError {
    #[error("redo record carries no header line")]
    MissingHeader,
    #[error("redo record header is not readable: {0}")]
    Header(String),
    #[error("redo record format version {0} is not supported")]
    UnsupportedVersion(u64),
    #[error("redo record holds fewer content bytes than its header declares")]
    Truncated,
    #[error("redo record carries {0} bytes beyond its last operation")]
    TrailingBytes(usize),
    #[error(transparent)]
    Path(#[from] RepositoryLayoutError),
}

/// The header line: every operation, with the length of the content bytes each
/// one claims from what follows.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordHeader {
    version: u64,
    puts: Vec<PutHeader>,
    deletes: Vec<PathHeader>,
    event_append: Option<AppendHeader>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PutHeader {
    root: RepositoryRootClass,
    relative: String,
    mode: FileMode,
    length: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathHeader {
    root: RepositoryRootClass,
    relative: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendHeader {
    offset: u64,
    length: u64,
}

/// The wire spelling of a path's root-relative portion.
fn relative_spelling(path: &VirtualPath) -> String {
    path.relative().as_str().to_owned()
}

/// Rebuild a canonical identity from its wire parts. The relative portion is
/// re-parsed rather than deserialized as its semantic type, so a spelling
/// escaping its root is rejected as a path rather than as a header defect.
fn canonical_path(
    root: RepositoryRootClass,
    relative: String,
) -> Result<VirtualPath, RedoRecordError> {
    Ok(VirtualPath::from_root(
        root,
        RootRelativePath::parse(relative)?,
    )?)
}

/// Claim the next `length` content bytes, advancing `read`. The claim is
/// bounds-checked, so a declared length no content backs yields
/// [`RedoRecordError::Truncated`] rather than an allocation.
fn take_content<'a>(
    content: &'a [u8],
    read: &mut usize,
    length: u64,
) -> Result<&'a [u8], RedoRecordError> {
    let end = usize::try_from(length)
        .ok()
        .and_then(|length| read.checked_add(length))
        .ok_or(RedoRecordError::Truncated)?;
    let taken = content.get(*read..end).ok_or(RedoRecordError::Truncated)?;
    *read = end;
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::temp_cleanup::cleanup_orphaned_temp_files;

    /// Marker naming the record file the parent process wrote. Reading it is a
    /// process-wide read of a variable only this test sets, and it is set on the
    /// child's environment alone, so the parent mutates no process-wide state.
    const CROSS_PROCESS_RECORD: &str = "JIT_TEST_REDO_RECORD_PATH";
    const CROSS_PROCESS_TEST: &str =
        "storage::redo_record::tests::test_parse_reads_a_record_another_process_serialized";

    /// A record covering both roots, both modes, non-UTF-8 content, a delete and
    /// an append — every operation the vocabulary carries.
    fn sample_record() -> RedoRecord {
        RedoRecord {
            puts: vec![
                PutFile {
                    path: VirtualPath::data("issues/a.json").unwrap(),
                    bytes: br#"{"id":"a"}"#.to_vec(),
                    mode: FileMode::Regular,
                },
                PutFile {
                    path: VirtualPath::worktree("scripts/publish.sh").unwrap(),
                    bytes: vec![0, 159, 146, 150, 255],
                    mode: FileMode::Executable,
                },
            ],
            deletes: vec![VirtualPath::worktree("dev/active/note.md").unwrap()],
            event_append: Some(EventAppend {
                offset: 41,
                bytes: b"{\"tag\":\"issue_created\"}\n".to_vec(),
            }),
        }
    }

    #[test]
    fn test_parse_returns_the_record_serialize_was_given() {
        let record = sample_record();
        assert_eq!(
            RedoRecord::parse(&record.serialize().unwrap()).unwrap(),
            record
        );
    }

    #[test]
    fn test_parse_returns_an_empty_record_for_a_mutation_that_changed_nothing() {
        let record = RedoRecord::default();
        assert_eq!(
            RedoRecord::parse(&record.serialize().unwrap()).unwrap(),
            record
        );
    }

    #[test]
    fn test_serialize_reproduces_the_bytes_parse_accepted() {
        let bytes = sample_record().serialize().unwrap();
        assert_eq!(
            RedoRecord::parse(&bytes).unwrap().serialize().unwrap(),
            bytes
        );
    }

    #[test]
    fn test_parse_reads_a_record_another_process_serialized() {
        // Child half: parse what the parent wrote and echo it back, so the
        // parent observes that a different process read the record.
        if let Ok(path) = std::env::var(CROSS_PROCESS_RECORD) {
            let record = RedoRecord::parse(&std::fs::read(&path).unwrap()).unwrap();
            std::fs::write(format!("{path}.echo"), record.serialize().unwrap()).unwrap();
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(RECORD_FILE_NAME);
        let written = sample_record().serialize().unwrap();
        std::fs::write(&path, &written).unwrap();

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CROSS_PROCESS_TEST])
            .env(CROSS_PROCESS_RECORD, &path)
            .status()
            .unwrap();

        assert!(status.success(), "the reading process failed");
        assert_eq!(
            std::fs::read(format!("{}.echo", path.display())).unwrap(),
            written,
            "the record another process parsed did not re-serialize to the same bytes"
        );
    }

    #[test]
    fn test_parse_rejects_a_path_outside_the_repository_roots() {
        let record = sample_record().serialize().unwrap();
        let end = record.iter().position(|byte| *byte == b'\n').unwrap();
        let header = String::from_utf8(record[..end].to_vec()).unwrap().replacen(
            "issues/a.json",
            "../outside/a.json",
            1,
        );
        let mut escaped = header.into_bytes();
        escaped.extend_from_slice(&record[end..]);

        assert!(matches!(
            RedoRecord::parse(&escaped),
            Err(RedoRecordError::Path(_))
        ));
    }

    #[test]
    fn test_parse_rejects_bytes_that_carry_no_header() {
        assert_eq!(
            RedoRecord::parse(b"not a redo record at all"),
            Err(RedoRecordError::MissingHeader)
        );
    }

    #[test]
    fn test_parse_rejects_content_shorter_than_the_header_declares() {
        let bytes = sample_record().serialize().unwrap();
        let truncated = &bytes[..bytes.len() - 1];

        assert_eq!(
            RedoRecord::parse(truncated),
            Err(RedoRecordError::Truncated)
        );
    }

    #[test]
    fn test_parse_rejects_bytes_beyond_the_last_operation() {
        let mut bytes = sample_record().serialize().unwrap();
        bytes.push(0);

        assert_eq!(
            RedoRecord::parse(&bytes),
            Err(RedoRecordError::TrailingBytes(1))
        );
    }

    #[test]
    fn test_record_file_name_survives_the_orphan_temp_sweep() {
        let scratch = tempfile::tempdir().unwrap();
        let directory = scratch.path().join(RECORD_SCRATCH_DIR);
        std::fs::create_dir(&directory).unwrap();
        let record = directory.join(RECORD_FILE_NAME);
        std::fs::write(&record, sample_record().serialize().unwrap()).unwrap();

        assert_eq!(cleanup_orphaned_temp_files(scratch.path(), 0).unwrap(), 0);
        assert!(record.exists(), "the sweep removed the record");
    }
}
