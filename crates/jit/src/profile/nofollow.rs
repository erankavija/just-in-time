//! The one no-follow open every route into a package reads through.
//!
//! Both routes — walking a package directory and capturing a tree from the
//! repository files a manifest declares — must answer two questions about the
//! same object: what its bytes are, and what kind and mode it carries. Asking a
//! pathname twice answers them about two objects whenever something replaces
//! the name in between, so the open here is the single lookup, and the caller
//! takes everything else from the handle it returns
//! (`@/inv/convention-convergence`).
//!
//! Containment comes from the same open. The handle is anchored at a directory
//! the caller already holds, and `cap_std` refuses a resolution that leaves it,
//! so an ancestor cannot be redirected out of that directory and a link at the
//! name itself is refused outright.

use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{
    Dir as CapDir, DirEntry as CapDirEntry, File as CapFile, Metadata as CapMetadata,
    OpenOptions as CapOpenOptions,
};
use std::path::Path;

/// Read options that refuse to follow a link into the name they open.
///
/// The open is non-blocking because opening is where an entry of the wrong kind
/// gets to make a decision on the reader's behalf: a pipe or a device opened for
/// reading waits for a peer, and a reader waiting forever rejects nothing. With
/// `O_NONBLOCK` the open returns at once and the caller's handle-metadata check
/// refuses the entry by name. The flag has no effect on the regular files a
/// package is made of, whose reads are unaffected by it.
fn read_options(maybe_dir: bool) -> CapOpenOptions {
    let mut options = CapOpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    options._cap_fs_ext_maybe_dir(maybe_dir);
    options._cap_fs_ext_nonblock(true);
    options
}

/// Open one listed entry through a no-follow `openat` on the handle that listed
/// it, so the object opened is the object listed or nothing at all.
pub(super) fn open_entry_nofollow(
    entry: &CapDirEntry,
    maybe_dir: bool,
) -> std::io::Result<CapFile> {
    entry.open_with(&read_options(maybe_dir))
}

/// Open `relative` beneath `directory` without following a link into the name,
/// and without leaving `directory` on the way to it.
pub(super) fn open_path_nofollow(
    directory: &CapDir,
    relative: &Path,
    maybe_dir: bool,
) -> std::io::Result<CapFile> {
    directory.open_with(relative, &read_options(maybe_dir))
}

/// What a failed no-follow open says about the name it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpenRefusal {
    /// The name is a symbolic link, which a no-follow open refuses outright.
    Symlink,
    /// The name cannot be opened as readable content at all: a device with no
    /// peer, or a link count the platform refuses.
    Unopenable,
    /// Resolving the name left the directory the open was anchored at.
    Escape,
    /// Ordinary I/O: the name is absent, unreadable, or the open failed for a
    /// reason that says nothing about what the name is.
    Io,
}

/// Classify one failed no-follow open.
///
/// The escape arm is the sandbox's own refusal, which carries no errno because
/// no system call reached the escaping path; every other arm is a platform
/// error code. Refusing to follow a symbolic link reports `ELOOP` under POSIX,
/// `EMLINK` on some BSDs, and a device with no peer reports `ENXIO`.
#[cfg(unix)]
pub(super) fn classify_open_failure(error: &std::io::Error) -> OpenRefusal {
    match error.raw_os_error() {
        Some(code) if code == nix::libc::ELOOP || code == nix::libc::EMLINK => OpenRefusal::Symlink,
        Some(code) if code == nix::libc::ENXIO => OpenRefusal::Unopenable,
        None if error.kind() == std::io::ErrorKind::PermissionDenied => OpenRefusal::Escape,
        _ => OpenRefusal::Io,
    }
}

/// Non-Unix: the errno classification above is POSIX- and BSD-specific, so only
/// the sandbox's own escape refusal is distinguishable.
#[cfg(not(unix))]
pub(super) fn classify_open_failure(error: &std::io::Error) -> OpenRefusal {
    if error.raw_os_error().is_none() && error.kind() == std::io::ErrorKind::PermissionDenied {
        OpenRefusal::Escape
    } else {
        OpenRefusal::Io
    }
}

/// Whether the opened file carries an executable Unix mode.
#[cfg(unix)]
pub(super) fn is_executable(metadata: &CapMetadata) -> bool {
    use cap_std::fs::PermissionsExt as _;

    metadata.permissions().mode() & 0o111 != 0
}

/// Windows has no executable mode bit for package-reader purposes.
#[cfg(not(unix))]
pub(super) fn is_executable(_metadata: &CapMetadata) -> bool {
    false
}
