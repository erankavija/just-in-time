//! Per-thread counts of the points at which a gate evaluation resolves its
//! inputs, keyed by the captured image resolved from, and for issues by the
//! record decoded. A gate definition is resolved with its registry.

use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Resolution {
    Issue,
    Registry,
    Documents,
    /// The repository index validation parses.
    Index,
}

thread_local! {
    static COUNTS: RefCell<HashMap<(Resolution, usize, String), usize>> = RefCell::new(HashMap::new());
}

pub(crate) fn count_in_image(
    resolution: Resolution,
    image: &crate::repository_state::RepositoryImage,
) {
    record(resolution, image.token().id(), String::new());
}

/// Count one decode of the issue record at `path` from `image`.
pub(crate) fn count_issue_in_image(
    image: &crate::repository_state::RepositoryImage,
    path: &crate::repository_state::VirtualPath,
) {
    record(
        Resolution::Issue,
        image.token().id(),
        path.repository_relative(),
    );
}

fn record(resolution: Resolution, image: usize, subject: String) {
    COUNTS.with(|counts| {
        *counts
            .borrow_mut()
            .entry((resolution, image, subject))
            .or_default() += 1
    });
}

/// For `resolution`, how many times each source (an image, or an image's
/// issue record) was resolved from on this thread.
pub(crate) fn per_source(resolution: Resolution) -> Vec<usize> {
    COUNTS.with(|counts| {
        counts
            .borrow()
            .iter()
            .filter(|((kind, ..), _)| *kind == resolution)
            .map(|(_, count)| *count)
            .collect()
    })
}

/// Forget every count, so the next ones cover the operation under test alone.
pub(crate) fn reset() {
    COUNTS.with(|counts| counts.borrow_mut().clear());
}
