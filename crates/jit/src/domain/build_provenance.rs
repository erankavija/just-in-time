//! Pure comparison of a running binary's build provenance against the
//! repository it is about to validate.
//!
//! Gate checkers invoke `jit` as an external tool (e.g. `scripts/jit-validate.sh`
//! runs `jit validate`). The `jit` resolved from `PATH` is whatever was last
//! installed, not necessarily a build of the working tree under review; when it
//! is stale, its verdict describes a different version of the product, yet
//! nothing distinguishes that from a trustworthy run (jit:7446af34). The binary
//! already knows the commit it was built from and whether that tree was dirty
//! ([`build_info::version_info`](crate::build_info::version_info)); this module
//! compares that against the repository's current `HEAD`.
//!
//! # The identity predicate (REQ-03)
//!
//! A bare inequality between the build commit and the repository's `HEAD` is
//! NOT enough to call a binary stale: an ordinary installed release run against
//! ANY unrelated git repository will almost always have a build commit that
//! differs from that repository's `HEAD` — the two share no history at all.
//! Flagging every such mismatch would misfire on the common case this check
//! must stay silent for.
//!
//! What actually distinguishes "this repository IS the one the binary was
//! built from, just at an older point in its history" from "this is an
//! unrelated repository" is whether the build commit is a commit object the
//! repository under validation actually contains. [`assess_binary_provenance`]
//! takes that as the caller-established `build_commit_known_in_repo` flag
//! (resolved via `git rev-parse --verify <commit>^{commit}` at the I/O
//! boundary — see [`commands::gate_check`](crate::commands) for the
//! production wiring) precisely so a plain string comparison never drives the
//! verdict.
//!
//! # Warn vs. fail (REQ-01)
//!
//! [`BinaryProvenance::Stale`] itself is silent on enforcement; the caller
//! ([`commands::gate_check::check_gate`](crate::commands)) turns it into a hard
//! failure (via `errors::StaleBinaryError`) rather than a warning. A gate
//! checker's verdict is meant to be evidence about the tree under review; a
//! verdict produced by a binary that predates that tree is not that evidence,
//! so there is nothing legitimate for a warning to preserve inside a gate
//! run — unlike an ad hoc `jit` invocation, a gate run's entire purpose is to
//! be trusted later. Failing also means no verdict is ever recorded from stale
//! evidence, so nothing downstream (`jit gate status`, rework loops) has to
//! second-guess whether a passed/failed run in the history is trustworthy.
//! The fix is one command away (`cargo install --path crates/jit`), so the
//! cost of failing loudly is low against the cost of a silently wrong verdict
//! (jit:7446af34's motivating incident: a stale binary reported a real fix as
//! "not found", and the `code-review` gate spent a review round chasing a
//! defect that did not exist in the working tree).

/// Why a running binary is judged to predate, or no longer match, the
/// repository it is validating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleBinaryReason {
    /// The binary was built from a commit other than the repository's
    /// current `HEAD`, and that build commit is a known commit in this
    /// repository's history.
    CommitMismatch {
        /// Full commit hash the running binary was built from.
        built_from: String,
        /// The repository's current `HEAD` commit hash.
        head: String,
    },
    /// The binary was built from a dirty working tree. A dirty build has no
    /// commit that fully describes its sources, so it can never be proven to
    /// still match the current tree — it is always treated as stale,
    /// regardless of whether the build commit equals `HEAD`.
    DirtyBuild {
        /// Full commit hash the running binary was built from (dirty on top
        /// of it).
        built_from: String,
    },
}

/// Outcome of comparing a running binary's build provenance against the
/// repository it is about to validate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryProvenance {
    /// The binary was built from the repository's current `HEAD`, from a
    /// clean tree: safe to trust as evidence about the tree under review.
    Fresh,
    /// The binary predates, or no longer matches, the tree under review.
    Stale(StaleBinaryReason),
    /// The comparison could not establish that the repository under
    /// validation shares the binary's build history (it contains the build
    /// commit — true of the source repository and of any clone or fork
    /// carrying that history) — no git, no resolvable `HEAD`, an unknown
    /// build commit (the binary was built outside a git checkout), or a
    /// build commit that is not a known commit in this repository's history.
    /// REQ-03: stay silent in every one of these cases rather than warn an
    /// ordinary user validating an unrelated repository.
    NotApplicable,
}

/// Compare a running binary's build commit/dirty flag against a repository's
/// current `HEAD`.
///
/// `build_commit_known_in_repo` is the REQ-03 identity predicate (see the
/// module docs): whether `build_commit` is a commit object the repository
/// under validation actually contains. The caller establishes it (e.g. via
/// `git rev-parse --verify <build_commit>^{commit}` against the repository
/// root) — this function performs no I/O of its own.
///
/// Returns [`BinaryProvenance::Stale`] when identity is established AND
/// either the build was dirty ([`StaleBinaryReason::DirtyBuild`], regardless
/// of whether `build_commit` equals `repo_head`) or `build_commit` differs
/// from `repo_head` ([`StaleBinaryReason::CommitMismatch`]). A build whose
/// dirty flag is `None` (unknown at build time) is not itself treated as
/// evidence of staleness — only a *confirmed* dirty build
/// (`Some(true)`) is.
pub fn assess_binary_provenance(
    build_commit: Option<&str>,
    build_dirty: Option<bool>,
    repo_head: Option<&str>,
    build_commit_known_in_repo: bool,
) -> BinaryProvenance {
    let (Some(build_commit), Some(repo_head)) = (build_commit, repo_head) else {
        return BinaryProvenance::NotApplicable;
    };
    if !build_commit_known_in_repo {
        return BinaryProvenance::NotApplicable;
    }
    if build_dirty == Some(true) {
        return BinaryProvenance::Stale(StaleBinaryReason::DirtyBuild {
            built_from: build_commit.to_string(),
        });
    }
    if build_commit != repo_head {
        return BinaryProvenance::Stale(StaleBinaryReason::CommitMismatch {
            built_from: build_commit.to_string(),
            head: repo_head.to_string(),
        });
    }
    BinaryProvenance::Fresh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_info;

    /// REQ-04 (matching case): the real compiled-in build commit, presented
    /// as this repository's own `HEAD` with a clean, known build, is `Fresh`.
    #[test]
    fn test_assess_binary_provenance_fresh_when_commit_matches_and_clean() {
        let real = build_info::version_info();
        let result = assess_binary_provenance(
            Some(real.git_commit),
            Some(false),
            Some(real.git_commit),
            true,
        );
        assert_eq!(result, BinaryProvenance::Fresh);
    }

    /// A build whose dirty flag is unknown (`None`) is not itself stale.
    #[test]
    fn test_assess_binary_provenance_fresh_when_dirty_unknown() {
        let real = build_info::version_info();
        let result =
            assess_binary_provenance(Some(real.git_commit), None, Some(real.git_commit), true);
        assert_eq!(result, BinaryProvenance::Fresh);
    }

    /// REQ-04 (mismatching case): the real compiled-in build commit, known in
    /// the repository but no longer at `HEAD`, is `Stale::CommitMismatch`.
    #[test]
    fn test_assess_binary_provenance_stale_on_commit_mismatch() {
        let real = build_info::version_info();
        let other_head = "0000000000000000000000000000000000000000";
        let result =
            assess_binary_provenance(Some(real.git_commit), Some(false), Some(other_head), true);
        assert_eq!(
            result,
            BinaryProvenance::Stale(StaleBinaryReason::CommitMismatch {
                built_from: real.git_commit.to_string(),
                head: other_head.to_string(),
            })
        );
    }

    /// A dirty build is stale even when its commit still equals `HEAD`: the
    /// commit alone does not describe the dirty changes on top of it.
    #[test]
    fn test_assess_binary_provenance_stale_when_dirty_even_if_commit_matches() {
        let real = build_info::version_info();
        let result = assess_binary_provenance(
            Some(real.git_commit),
            Some(true),
            Some(real.git_commit),
            true,
        );
        assert_eq!(
            result,
            BinaryProvenance::Stale(StaleBinaryReason::DirtyBuild {
                built_from: real.git_commit.to_string(),
            })
        );
    }

    /// REQ-03: no resolvable `HEAD` (no git, or a git repo with no commits)
    /// stays silent.
    #[test]
    fn test_assess_binary_provenance_not_applicable_without_repo_head() {
        let real = build_info::version_info();
        assert_eq!(
            assess_binary_provenance(Some(real.git_commit), Some(false), None, false),
            BinaryProvenance::NotApplicable
        );
    }

    /// A binary built outside a git checkout has no build commit to compare.
    #[test]
    fn test_assess_binary_provenance_not_applicable_without_build_commit() {
        assert_eq!(
            assess_binary_provenance(None, None, Some("deadbeef"), false),
            BinaryProvenance::NotApplicable
        );
    }

    /// REQ-03: a commit mismatch alone never implies staleness — only a
    /// mismatch where the build commit is ALSO a known commit in this
    /// repository does. Otherwise every unrelated repository would misfire.
    #[test]
    fn test_assess_binary_provenance_not_applicable_when_repo_unrelated() {
        let real = build_info::version_info();
        assert_eq!(
            assess_binary_provenance(Some(real.git_commit), Some(false), Some("cafef00d"), false),
            BinaryProvenance::NotApplicable
        );
    }
}
