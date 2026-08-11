//! The one reproducible tar header every archive this project writes is built
//! from.
//!
//! Ownership, timestamps, and permissions are the header fields a tar writer
//! would otherwise take from the machine it runs on, which is what makes two
//! runs over identical content produce different bytes. Fixing them in one
//! place is what lets a snapshot and a profile package archive both claim that
//! their bytes are a function of their content rather than of when and where
//! they were written (`@/inv/convention-convergence`).

use tar::{EntryType, Header};

/// Build one tar header carrying no per-run identity.
///
/// Ownership is root, the modification time is the epoch, and `mode` is the
/// caller's declaration rather than a source file's permissions, so the header
/// says nothing about the machine that wrote it. `size` is the entry's content
/// length, zero for a directory.
///
/// The checksum is computed here so the header is complete on return; a writer
/// that also sets the entry's path recomputes it over the final field set.
pub(crate) fn reproducible_tar_header(entry_type: EntryType, mode: u32, size: u64) -> Header {
    let mut header = Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(size);
    header.set_cksum();
    header
}
