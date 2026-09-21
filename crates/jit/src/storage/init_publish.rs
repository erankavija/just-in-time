//! Publication of a data root that does not exist yet.
//!
//! The complete new root is written into a sibling stage directory, synced, and
//! published with a no-replace rename, so an occupied destination is refused
//! rather than overwritten (`@/inv/atomic-writes`, OD-6). No journal precedes
//! it: the record's home is inside the root that does not exist yet, so an
//! interruption before the rename leaves no root at all, and the orphan stage
//! it left is removed by the next publication.

/// The stage directory's name beside the data root named `data_leaf`.
///
/// One deterministic name, so the next publication reclaims the orphan an
/// interrupted one left; the session's bootstrap lock is what keeps two
/// publications from sharing it.
pub(in crate::storage) fn stage_name(data_leaf: &str) -> String {
    format!("{data_leaf}.stage")
}
