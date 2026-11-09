//! Per-thread counts of the points at which a gate evaluation resolves its
//! inputs, keyed by the captured image resolved from. A gate definition is
//! resolved with its registry.

use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Resolution {
    Issue,
    Registry,
    Documents,
}

thread_local! {
    static COUNTS: RefCell<HashMap<(Resolution, usize), usize>> = RefCell::new(HashMap::new());
}

pub(crate) fn count_in_image(
    resolution: Resolution,
    image: &crate::repository_state::RepositoryImage,
) {
    record(resolution, image.token().id());
}

fn record(resolution: Resolution, key: usize) {
    COUNTS.with(|counts| *counts.borrow_mut().entry((resolution, key)).or_default() += 1);
}

/// For `resolution`, how many times each source was resolved from on this thread.
pub(crate) fn per_source(resolution: Resolution) -> Vec<usize> {
    COUNTS.with(|counts| {
        counts
            .borrow()
            .iter()
            .filter(|((kind, _), _)| *kind == resolution)
            .map(|(_, count)| *count)
            .collect()
    })
}

/// Forget every count, so the next ones cover the operation under test alone.
pub(crate) fn reset() {
    COUNTS.with(|counts| counts.borrow_mut().clear());
}
