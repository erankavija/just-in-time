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
//! compares that against the repository's current `HEAD` and the paths that
//! changed between them.
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
//! boundary — see `CommandExecutor::stale_binary_reason` (`commands::gate_check`)
//! for the production wiring, shared by both call sites below) precisely so a
//! plain string comparison never drives the verdict. The full refusal
//! condition is therefore always both parts together: (1) this identity
//! predicate holds, AND (2) [`assess_binary_provenance`] finds either a
//! build-input change or a dirty build; a repository that fails part (1) —
//! unrelated, no git, or an unresolvable build commit — never refuses,
//! regardless of part (2).
//!
//! # Warn vs. fail (REQ-01)
//!
//! [`BinaryProvenance::Stale`] itself is silent on enforcement; both
//! production callers of `CommandExecutor::stale_binary_reason` turn it into
//! a hard failure (via `errors::StaleBinaryError`) rather than a warning:
//! [`commands::gate_check::check_gate`](crate::commands) (the evaluator's own
//! guard, before it spawns a checker) and `main`'s startup dispatch (a
//! checker's own child `jit` self-checking, REQ-02 — a checker script that
//! shells out to `jit` resolves it from `PATH` independently of the
//! evaluator). A gate checker's verdict is meant to be evidence about the
//! tree under review; a verdict produced by a binary that predates that tree
//! is not that evidence, so there is nothing legitimate for a warning to
//! preserve inside a gate run — unlike an ad hoc `jit` invocation, a gate
//! run's entire purpose is to be trusted later. Failing also means the
//! refusing process itself never turns the stale binary's own computation
//! into a recorded verdict (when `check_gate` refuses, no gate run is
//! recorded at all; when a checker's child refuses instead, the evaluator
//! still records an ordinary failed run around it, but that run's content is
//! the refusal, never the stale binary's actual output), so nothing
//! downstream (`jit gate status`, rework loops) has to second-guess whether a
//! passed/failed run in the history is trustworthy. The fix is one command
//! away (`cargo install --path crates/jit`), so the cost of failing loudly is
//! low against the cost of a silently wrong verdict (jit:7446af34's
//! motivating incident: a stale binary reported a real fix as "not found",
//! and the `code-review` gate spent a review round chasing a defect that did
//! not exist in the working tree).

use crate::domain::repository_inputs::RepositoryInputs;

/// Why a running binary is judged to predate, or no longer match, the
/// repository it is validating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleBinaryReason {
    /// The repository has changed a path that can affect the binary since the
    /// recorded build commit. This also represents a build-input change in an
    /// otherwise unchanged `HEAD`, because the commit alone then no longer
    /// describes the tree the gate is reviewing.
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

/// Paths that are actual inputs to the production `jit` binary.
///
/// This is a positive build-input inventory, not a denylist of paths that are
/// presumed safe. The source and manifest roots cover Cargo's compilation
/// inputs; the profile and hook paths cover the files embedded by
/// `include_dir!`/`include_str!` in production code. A newly added path
/// elsewhere in the repository therefore remains irrelevant by construction.
const BINARY_BUILD_INPUT_ROOTS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "crates/jit/Cargo.toml",
    "crates/jit/Cargo.lock",
    "crates/jit/build.rs",
    "crates/jit/src",
    "profiles/jit-dogfood",
    "scripts/hooks/pre-commit",
    "scripts/hooks/pre-push",
];

/// The binary's build inputs as a declared input set.
///
/// The same declaration shape a quality gate states its checker's inputs in,
/// so one covering rule serves both questions
/// (`@/invariant/convention-convergence`). Built once and reused; the
/// inventory above is fixed source text, and
/// `test_binary_build_input_inventory_ignores_unrelated_paths` fails outright
/// if an entry stops parsing, so the `None` arm cannot ship silently.
static BINARY_BUILD_INPUTS: std::sync::LazyLock<Option<RepositoryInputs>> =
    std::sync::LazyLock::new(|| RepositoryInputs::parse(BINARY_BUILD_INPUT_ROOTS, &[]).ok());

/// Whether `path` can affect the production `jit` binary.
pub fn is_binary_build_input(path: &str) -> bool {
    BINARY_BUILD_INPUTS
        .as_ref()
        .is_some_and(|inputs| inputs.covers(path))
}

/// Whether any changed repository path can affect the production `jit`
/// binary.
pub fn binary_build_inputs_changed<I, P>(paths: I) -> bool
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    paths
        .into_iter()
        .any(|path| is_binary_build_input(path.as_ref()))
}

/// Compare a running binary's build commit/dirty flag against a repository's
/// current `HEAD` and its changed build-input paths.
///
/// `build_commit_known_in_repo` is the REQ-03 identity predicate (see the
/// module docs): whether `build_commit` is a commit object the repository
/// under validation actually contains. The caller establishes it (e.g. via
/// `git rev-parse --verify <build_commit>^{commit}` against the repository
/// root) — this function performs no I/O of its own.
///
/// `build_inputs_changed` is supplied by the I/O boundary after comparing the
/// committed and working-tree path changes against
/// [`is_binary_build_input`]. Returns [`BinaryProvenance::Stale`] when
/// identity is established AND either the build was dirty
/// ([`StaleBinaryReason::DirtyBuild`], regardless of whether `build_commit`
/// equals `repo_head`) or a build-input path changed. A build whose dirty flag
/// is `None` (unknown at build time) is not itself treated as evidence of
/// staleness — only a *confirmed* dirty build (`Some(true)`) is.
pub fn assess_binary_provenance(
    build_commit: Option<&str>,
    build_dirty: Option<bool>,
    repo_head: Option<&str>,
    build_commit_known_in_repo: bool,
    build_inputs_changed: bool,
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
    if build_inputs_changed {
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
            false,
        );
        assert_eq!(result, BinaryProvenance::Fresh);
    }

    /// A build whose dirty flag is unknown (`None`) is not itself stale.
    #[test]
    fn test_assess_binary_provenance_fresh_when_dirty_unknown() {
        let real = build_info::version_info();
        let result = assess_binary_provenance(
            Some(real.git_commit),
            None,
            Some(real.git_commit),
            true,
            false,
        );
        assert_eq!(result, BinaryProvenance::Fresh);
    }

    /// REQ-04 (mismatching case): the real compiled-in build commit, known in
    /// the repository but no longer at `HEAD`, is `Stale::CommitMismatch`.
    #[test]
    fn test_assess_binary_provenance_stale_on_commit_mismatch() {
        let real = build_info::version_info();
        let other_head = "0000000000000000000000000000000000000000";
        let result = assess_binary_provenance(
            Some(real.git_commit),
            Some(false),
            Some(other_head),
            true,
            true,
        );
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
            false,
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
            assess_binary_provenance(Some(real.git_commit), Some(false), None, false, false),
            BinaryProvenance::NotApplicable
        );
    }

    /// A binary built outside a git checkout has no build commit to compare.
    #[test]
    fn test_assess_binary_provenance_not_applicable_without_build_commit() {
        assert_eq!(
            assess_binary_provenance(None, None, Some("deadbeef"), false, true),
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
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                Some("cafef00d"),
                false,
                true,
            ),
            BinaryProvenance::NotApplicable
        );
    }

    #[test]
    fn test_binary_build_input_inventory_ignores_unrelated_paths() {
        assert!(
            RepositoryInputs::parse(BINARY_BUILD_INPUT_ROOTS, &[]).is_ok(),
            "every build-input entry must parse as a declared root"
        );
        assert!(is_binary_build_input("Cargo.toml"));
        assert!(is_binary_build_input("crates/jit/src/main.rs"));
        assert!(is_binary_build_input("profiles/jit-dogfood/manifest.toml"));
        assert!(is_binary_build_input("scripts/hooks/pre-commit"));
        assert!(!is_binary_build_input("docs/new-reference.md"));
        assert!(!is_binary_build_input("scripts/new-tool.sh"));
        assert!(!binary_build_inputs_changed([
            "docs/new-reference.md",
            "scripts/new-tool.sh",
        ]));
    }

    #[test]
    fn test_assess_binary_provenance_fresh_for_metadata_only_changes() {
        let real = build_info::version_info();
        assert_eq!(
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                Some("a-different-known-head"),
                true,
                false,
            ),
            BinaryProvenance::Fresh
        );
    }

    #[test]
    fn test_assess_binary_provenance_stale_for_same_head_build_input_change() {
        let real = build_info::version_info();
        assert_eq!(
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                Some(real.git_commit),
                true,
                true,
            ),
            BinaryProvenance::Stale(StaleBinaryReason::CommitMismatch {
                built_from: real.git_commit.to_string(),
                head: real.git_commit.to_string(),
            })
        );
    }
}
