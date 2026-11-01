//! Test-only counters for the stages a profile selection runs once per
//! invocation.
//!
//! Each stage records itself at the one site that performs it, under
//! `#[cfg(test)]`, so production builds carry neither the counters nor a second
//! code path. Counts are thread-local because a command invocation runs its
//! selection on the calling thread, which keeps parallel tests independent.

use std::cell::RefCell;
use std::collections::BTreeMap;

/// One stage of building and running a profile selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SelectionStage {
    /// Reading the packages a selection's selectors name.
    SelectorResolution,
    /// Loading the selection's dependencies and resolving its package graph.
    GraphResolution,
    /// Validating variable inputs against the declarations they may name.
    VariableValidation,
    /// Validating one package model's variable references.
    ModelReferenceValidation,
    /// Composing one package's resolved content into claims.
    PackageComposition,
}

thread_local! {
    static COUNTS: RefCell<Option<BTreeMap<SelectionStage, usize>>> = const { RefCell::new(None) };
}

/// Count one run of `stage` while an [`observe`] call is active on this thread.
pub(crate) fn record(stage: SelectionStage) {
    COUNTS.with(|counts| {
        if let Some(counts) = counts.borrow_mut().as_mut() {
            *counts.entry(stage).or_default() += 1;
        }
    });
}

/// Run `operation` and return its value with how often it ran each stage.
pub(crate) fn observe<T>(operation: impl FnOnce() -> T) -> (T, BTreeMap<SelectionStage, usize>) {
    COUNTS.with(|counts| *counts.borrow_mut() = Some(BTreeMap::new()));
    let value = operation();
    let counts = COUNTS.with(|counts| counts.borrow_mut().take().unwrap_or_default());
    (value, counts)
}

/// The counts one invocation over a closure of `closure` packages runs: each
/// selection-wide stage once, and each per-package stage once per member.
pub(crate) fn once_per_invocation(closure: usize) -> BTreeMap<SelectionStage, usize> {
    BTreeMap::from([
        (SelectionStage::SelectorResolution, 1),
        (SelectionStage::GraphResolution, 1),
        (SelectionStage::VariableValidation, 1),
        (SelectionStage::ModelReferenceValidation, closure),
        (SelectionStage::PackageComposition, closure),
    ])
}
