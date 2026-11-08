//! Closed [`RepositoryImage`]s built from declared captures, with no storage
//! backend or mutation session.

use crate::repository_state::{
    CaptureBudget, CaptureSpec, CapturedBytes, DirectoryListing, EventLogTail, FileMode,
    PinnedDocumentEvidence, RepositoryEntry, RepositoryImage, RepositoryLayout,
    RepositoryRootEvidence, VirtualPath,
};
use std::collections::BTreeMap;

/// The fixed layout of a synthetic repository at `/repo` with data at `/repo/.jit`.
pub fn synthetic_layout() -> RepositoryLayout {
    RepositoryLayout::new(
        RepositoryRootEvidence::new("/repo", "worktree", true),
        RepositoryRootEvidence::new("/repo/.jit", "data", true),
    )
    .unwrap()
}

/// A path an [`ImageBuilder`] accepts: a [`VirtualPath`], or a
/// repository-relative spelling classified against the builder's layout.
pub trait ImagePath {
    /// The virtual path this names under `layout`.
    fn resolve(self, layout: &RepositoryLayout) -> VirtualPath;
}

impl ImagePath for VirtualPath {
    fn resolve(self, _layout: &RepositoryLayout) -> VirtualPath {
        self
    }
}

impl ImagePath for &str {
    fn resolve(self, layout: &RepositoryLayout) -> VirtualPath {
        layout.classify_repository_relative(self).unwrap()
    }
}

/// Declares entries, listings and pinned evidence, then closes an image over
/// them.
///
/// Unless [`with_spec`](Self::with_spec) supplies one, the capture spec requests
/// exactly what was declared, each in the read mode its answer implies, within
/// an unbounded budget. Event-log files are reduced to their tail, and an
/// undeclared event log is captured absent, as every publishing capture reads
/// the log its plan may append to.
#[derive(Debug)]
pub struct ImageBuilder {
    layout: RepositoryLayout,
    spec: Option<CaptureSpec>,
    entries: BTreeMap<VirtualPath, RepositoryEntry>,
    listings: BTreeMap<VirtualPath, DirectoryListing>,
    pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
}

impl Default for ImageBuilder {
    fn default() -> Self {
        Self::over(synthetic_layout())
    }
}

impl ImageBuilder {
    /// A builder over [`synthetic_layout`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder over `layout`.
    pub fn over(layout: RepositoryLayout) -> Self {
        Self {
            layout,
            spec: None,
            entries: BTreeMap::new(),
            listings: BTreeMap::new(),
            pinned: BTreeMap::new(),
        }
    }

    /// Capture `path` as `entry`.
    pub fn entry(mut self, path: impl ImagePath, entry: RepositoryEntry) -> Self {
        let path = path.resolve(&self.layout);
        let entry = match entry {
            RepositoryEntry::File { content, mode } if path == VirtualPath::EVENTS => {
                RepositoryEntry::FileTail {
                    tail: EventLogTail::of_bytes(content.bytes()),
                    mode,
                }
            }
            entry => entry,
        };
        self.entries.insert(path, entry);
        self
    }

    /// Capture a regular file with `bytes`.
    pub fn file(self, path: impl ImagePath, bytes: impl AsRef<[u8]>) -> Self {
        let path = path.resolve(&self.layout);
        let entry = RepositoryEntry::File {
            content: CapturedBytes::new(path.repository_relative(), bytes.as_ref().to_vec())
                .unwrap(),
            mode: FileMode::Regular,
        };
        self.entry(path, entry)
    }

    /// Capture each repository-relative path as a regular file holding its
    /// text, or as absent when the text is `None`.
    pub fn files(self, files: &[(&str, Option<&str>)]) -> Self {
        files.iter().fold(self, |builder, (path, text)| match text {
            Some(text) => builder.file(*path, text),
            None => builder.entry(*path, RepositoryEntry::Absent),
        })
    }

    /// Capture `path`'s complete listing.
    pub fn listing(mut self, path: impl ImagePath, listing: DirectoryListing) -> Self {
        self.listings.insert(path.resolve(&self.layout), listing);
        self
    }

    /// Capture `evidence` as the answer to the pinned `(revision, path)` request.
    pub fn pinned(
        mut self,
        revision: impl Into<String>,
        path: impl Into<String>,
        evidence: PinnedDocumentEvidence,
    ) -> Self {
        self.pinned.insert((revision.into(), path.into()), evidence);
        self
    }

    /// Close the image over `spec` instead of the derived one.
    pub fn with_spec(mut self, spec: CaptureSpec) -> Self {
        self.spec = Some(spec);
        self
    }

    /// The closed image; panics when the declarations do not close.
    pub fn build(mut self) -> RepositoryImage {
        let reads_events = self
            .spec
            .as_ref()
            .is_none_or(|spec| spec.contains_path(&VirtualPath::EVENTS));
        if reads_events {
            self.entries
                .entry(VirtualPath::EVENTS)
                .or_insert(RepositoryEntry::Absent);
        }
        let spec = self
            .spec
            .unwrap_or_else(|| derived_spec(&self.entries, &self.listings, &self.pinned));
        RepositoryImage::close(
            self.layout,
            spec,
            self.entries,
            self.listings,
            self.pinned,
            &Default::default(),
        )
        .unwrap()
    }
}

fn derived_spec(
    entries: &BTreeMap<VirtualPath, RepositoryEntry>,
    listings: &BTreeMap<VirtualPath, DirectoryListing>,
    pinned: &BTreeMap<(String, String), PinnedDocumentEvidence>,
) -> CaptureSpec {
    let budget = CaptureBudget {
        max_listings: usize::MAX,
        max_bytes: u64::MAX,
        max_depth: usize::MAX,
    };
    let mut spec = CaptureSpec::new([], budget).unwrap();
    for (path, entry) in entries {
        if *path == VirtualPath::EVENTS {
            spec.discover_tail(path.clone()).unwrap();
            continue;
        }
        match entry {
            RepositoryEntry::FileTail { .. } => spec.discover_tail(path.clone()),
            RepositoryEntry::FileKind { .. } => spec.discover_kinds([path.clone()]),
            _ => spec.discover_paths([path.clone()]),
        }
        .unwrap();
    }
    spec.discover_listings(listings.keys().cloned()).unwrap();
    for ((revision, path), evidence) in pinned {
        if evidence.exists() && evidence.bytes().is_none() {
            spec.discover_pinned_kind(revision, path)
        } else {
            spec.discover_pinned(revision, path)
        }
        .unwrap();
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{
        CaptureError, EntryIdentity, EventLogTail, PinnedSourceClass, ReadMode,
        RepositoryTargetKind,
    };

    #[test]
    fn test_image_builder_captures_file_bytes_with_regular_mode() {
        let image = ImageBuilder::new()
            .file(VirtualPath::CONFIG, b"config")
            .build();

        assert!(matches!(
            image.entry(&VirtualPath::CONFIG).unwrap(),
            RepositoryEntry::File { content, mode: FileMode::Regular }
                if content.bytes() == b"config"
        ));
    }

    #[test]
    fn test_image_builder_captures_event_log_bytes_as_a_tail() {
        let log = b"line\n".repeat(2000);
        let image = ImageBuilder::new().file(VirtualPath::EVENTS, &log).build();

        assert!(matches!(
            image.entry(&VirtualPath::EVENTS).unwrap(),
            RepositoryEntry::FileTail { tail, .. } if *tail == EventLogTail::of_bytes(&log)
        ));
    }

    #[test]
    fn test_image_builder_reads_back_each_declared_entry_kind() {
        let directory = RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("dir", b"dir").unwrap(),
            mode: FileMode::Regular,
        };
        let symlink = RepositoryEntry::Symlink {
            target: CapturedBytes::new("link", b"target".to_vec()).unwrap(),
            mode: FileMode::Regular,
        };
        let tail = RepositoryEntry::FileTail {
            tail: EventLogTail::of_bytes(b"line\n"),
            mode: FileMode::Regular,
        };
        let declared = [
            (
                VirtualPath::worktree("docs/a.md").unwrap(),
                RepositoryEntry::File {
                    content: CapturedBytes::new("file", b"bytes".to_vec()).unwrap(),
                    mode: FileMode::Regular,
                },
            ),
            (
                VirtualPath::data("absent.json").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::worktree("docs").unwrap(), directory),
            (VirtualPath::worktree("link").unwrap(), symlink),
            (VirtualPath::EVENTS, tail),
        ];
        let image = declared
            .iter()
            .cloned()
            .fold(ImageBuilder::new(), |builder, (path, entry)| {
                builder.entry(path, entry)
            })
            .build();

        for (path, entry) in &declared {
            assert_eq!(image.entry(path).unwrap(), entry);
        }
        assert_eq!(
            image.file_bytes(&declared[0].0).unwrap(),
            Some(&b"bytes"[..])
        );
        assert_eq!(
            image.capture_spec().read_mode(&VirtualPath::EVENTS),
            Some(ReadMode::Tail)
        );
    }

    #[test]
    fn test_image_builder_classifies_repository_relative_paths_against_its_layout() {
        let image = ImageBuilder::new()
            .file(".jit/config.toml", b"config")
            .entry("docs/a.md", RepositoryEntry::Absent)
            .build();

        assert_eq!(
            image.file_bytes(&VirtualPath::CONFIG).unwrap(),
            Some(&b"config"[..])
        );
        assert_eq!(
            image
                .file_bytes(&VirtualPath::worktree("docs/a.md").unwrap())
                .unwrap(),
            None
        );
    }

    #[test]
    fn test_image_builder_files_captures_text_as_files_and_none_as_absent() {
        let absent = VirtualPath::worktree("docs/a.md").unwrap();
        let image = ImageBuilder::new()
            .files(&[(".jit/config.toml", Some("config")), ("docs/a.md", None)])
            .build();

        assert_eq!(
            image.file_bytes(&VirtualPath::CONFIG).unwrap(),
            Some(&b"config"[..])
        );
        assert_eq!(image.entry(&absent).unwrap(), &RepositoryEntry::Absent);
    }

    #[test]
    fn test_image_builder_reads_back_a_listing_with_its_names() {
        let path = VirtualPath::worktree("docs").unwrap();
        let listing = DirectoryListing::new(["a.md".into(), "b.md".into()]).unwrap();
        let image = ImageBuilder::new()
            .listing(path.clone(), listing.clone())
            .build();

        assert_eq!(image.directory_listings().get(&path), Some(&listing));
    }

    #[test]
    fn test_image_builder_reads_back_pinned_evidence_by_revision_and_path() {
        let bytes = b"pinned".to_vec();
        let evidence = PinnedDocumentEvidence::new(
            "HEAD",
            "docs/a.md",
            PinnedSourceClass::GitObject,
            RepositoryTargetKind::File,
            Some("a".repeat(40)),
            Some("b".repeat(40)),
            Some(EntryIdentity::for_bytes("blob", &bytes).unwrap()),
            Some(bytes),
            None,
        )
        .unwrap();
        let image = ImageBuilder::new()
            .pinned("HEAD", "docs/a.md", evidence.clone())
            .build();

        assert_eq!(
            image
                .pinned_evidence()
                .get(&("HEAD".to_string(), "docs/a.md".to_string())),
            Some(&evidence)
        );
    }

    #[test]
    fn test_image_builder_closes_over_a_caller_supplied_spec_and_layout() {
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/elsewhere", "worktree", true),
            RepositoryRootEvidence::new("/elsewhere/.jit", "data", true),
        )
        .unwrap();
        let budget = CaptureBudget {
            max_listings: 0,
            max_bytes: 64,
            max_depth: 2,
        };
        let mut spec = CaptureSpec::new([], budget).unwrap();
        spec.discover_kinds([VirtualPath::CONFIG]).unwrap();
        let image = ImageBuilder::over(layout.clone())
            .with_spec(spec.clone())
            .entry(
                VirtualPath::CONFIG,
                RepositoryEntry::FileKind {
                    mode: FileMode::Regular,
                },
            )
            .build();

        assert_eq!(image.capture_spec(), &spec);
        assert_eq!(image.layout(), &layout);
    }

    #[test]
    fn test_image_builder_refuses_a_path_outside_the_declared_set_as_uncaptured() {
        let undeclared = VirtualPath::worktree("other.md").unwrap();
        let image = ImageBuilder::new().file("docs/a.md", b"bytes").build();

        assert!(matches!(
            image.entry(&undeclared),
            Err(CaptureError::UndiscoveredRepositoryPath(path)) if path == undeclared
        ));
    }
}
