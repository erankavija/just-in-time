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
//! predicate holds, AND (2) [`assess_binary_provenance`] finds a committed
//! build-input change, an uncommitted build-input change, or a dirty build.
//! The resulting reason carries the actionable evidence: both commits for a
//! committed change, the changed paths for an uncommitted change, or the
//! build commit for a dirty build.
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

/// Why a running binary's build does not match the repository it is
/// validating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleBinaryReason {
    /// The repository's committed history has changed a path that can affect
    /// the binary since the recorded build commit.
    CommitMismatch {
        /// Full commit hash the running binary was built from.
        built_from: String,
        /// The repository's current `HEAD` commit hash.
        head: String,
    },
    /// The repository has uncommitted changes to build inputs. The paths are
    /// kept so the refusal can identify what must be committed or reverted;
    /// unlike [`CommitMismatch`], this reason does not report the current
    /// `HEAD` as a conflicting commit.
    UncommittedBuildInputs {
        /// Full commit hash the running binary was built from.
        built_from: String,
        /// Build-input paths changed in the working tree.
        paths: Vec<String>,
    },
    /// The binary was built from a tree carrying an uncommitted build input.
    /// Such a build has no commit that fully describes its sources, so it can
    /// never be proven to still match the current tree — it is always treated
    /// as stale, regardless of whether the build commit equals `HEAD`. What
    /// counts as a build input is the inventory below, so an uncommitted file
    /// that feeds no build never reaches this state.
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

/// The inventory of the binary's build inputs, verbatim.
///
/// Compiled in from `binary_build_inputs.txt` rather than written here, because
/// `scripts/install-jit.sh` reads the same file to decide whether the tree it
/// is installing from carries uncommitted build inputs. One inventory, two
/// consumers, neither restating it (REQ-12). Its own header states the format
/// and why it is shared; embedding it also makes the file a build input of the
/// binary in the ordinary way, so editing it rebuilds what reads it.
const BINARY_BUILD_INPUT_DECLARATION: &str = include_str!("binary_build_inputs.txt");

/// The inventory's lines, comments and blanks removed.
///
/// The parsing is the file's stated format and nothing more, so the installer's
/// reading of the same file cannot disagree about which lines are entries.
pub(crate) fn declared_build_input_roots() -> Vec<&'static str> {
    BINARY_BUILD_INPUT_DECLARATION
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// The binary's build inputs as a declared input set.
///
/// The same declaration shape a quality gate states its checker's inputs in,
/// resolved by the same covering rule, so one rule answers both questions
/// (REQ-12, `@/invariant/convention-convergence`). Built once and reused; the
/// inventory is fixed source text, and
/// `test_binary_build_input_inventory_ignores_unrelated_paths` fails outright
/// if an entry stops parsing, so the `None` arm cannot ship silently.
static BINARY_BUILD_INPUTS: std::sync::LazyLock<Option<RepositoryInputs>> =
    std::sync::LazyLock::new(|| RepositoryInputs::parse(&declared_build_input_roots(), &[]).ok());

/// Whether `path` can affect the production `jit` binary.
pub fn is_binary_build_input(path: &str) -> bool {
    BINARY_BUILD_INPUTS
        .as_ref()
        .is_some_and(|inputs| inputs.covers(path))
}

/// Changed build-input paths, separated by whether they are committed or
/// only present in the working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildInputChanges {
    /// Build-input paths changed between the binary's build commit and `HEAD`.
    pub committed: Vec<String>,
    /// Build-input paths changed in the working tree relative to `HEAD`.
    pub working_tree: Vec<String>,
}

/// Select the changed paths that can affect the production `jit` binary while
/// retaining whether each came from committed history or the working tree.
pub fn binary_build_input_changes<CI, CP, WI, WP>(
    committed_paths: CI,
    working_tree_paths: WI,
) -> BuildInputChanges
where
    CI: IntoIterator<Item = CP>,
    CP: AsRef<str>,
    WI: IntoIterator<Item = WP>,
    WP: AsRef<str>,
{
    let committed = committed_paths
        .into_iter()
        .filter(|path| is_binary_build_input(path.as_ref()))
        .map(|path| path.as_ref().to_string())
        .collect();
    let working_tree = working_tree_paths
        .into_iter()
        .filter(|path| is_binary_build_input(path.as_ref()))
        .map(|path| path.as_ref().to_string())
        .collect();
    BuildInputChanges {
        committed,
        working_tree,
    }
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
/// `build_input_changes` is supplied by the I/O boundary after comparing the
/// committed and working-tree path changes against
/// [`is_binary_build_input`]. Returns [`BinaryProvenance::Stale`] when
/// identity is established AND either the build was dirty
/// ([`StaleBinaryReason::DirtyBuild`], regardless of whether `build_commit`
/// equals `repo_head`), an uncommitted build-input path changed, or a committed
/// build-input path changed. A build whose dirty flag is `None` (unknown at
/// build time) is not itself treated as evidence of staleness — only a
/// *confirmed* dirty build (`Some(true)`) is.
pub fn assess_binary_provenance(
    build_commit: Option<&str>,
    build_dirty: Option<bool>,
    repo_head: Option<&str>,
    build_commit_known_in_repo: bool,
    build_input_changes: &BuildInputChanges,
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
    if !build_input_changes.working_tree.is_empty() {
        return BinaryProvenance::Stale(StaleBinaryReason::UncommittedBuildInputs {
            built_from: build_commit.to_string(),
            paths: build_input_changes.working_tree.clone(),
        });
    }
    if !build_input_changes.committed.is_empty() {
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
            &BuildInputChanges::default(),
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
            &BuildInputChanges::default(),
        );
        assert_eq!(result, BinaryProvenance::Fresh);
    }

    /// REQ-04 (mismatching case): the real compiled-in build commit, known in
    /// the repository but no longer at `HEAD`, is `Stale::CommitMismatch`.
    #[test]
    fn test_assess_binary_provenance_stale_on_commit_mismatch() {
        let real = build_info::version_info();
        let other_head = "0000000000000000000000000000000000000000";
        let build_input_changes =
            binary_build_input_changes(["crates/jit/src/main.rs"], std::iter::empty::<&str>());
        let result = assess_binary_provenance(
            Some(real.git_commit),
            Some(false),
            Some(other_head),
            true,
            &build_input_changes,
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
            &BuildInputChanges::default(),
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
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                None,
                false,
                &BuildInputChanges::default(),
            ),
            BinaryProvenance::NotApplicable
        );
    }

    /// A binary built outside a git checkout has no build commit to compare.
    #[test]
    fn test_assess_binary_provenance_not_applicable_without_build_commit() {
        assert_eq!(
            assess_binary_provenance(
                None,
                None,
                Some("deadbeef"),
                false,
                &BuildInputChanges::default(),
            ),
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
                &BuildInputChanges::default(),
            ),
            BinaryProvenance::NotApplicable
        );
    }

    #[test]
    fn test_binary_build_input_inventory_ignores_unrelated_paths() {
        assert!(
            RepositoryInputs::parse(&declared_build_input_roots(), &[]).is_ok(),
            "every build-input entry must parse as a declared root"
        );
        assert!(is_binary_build_input("Cargo.toml"));
        assert!(is_binary_build_input("crates/jit/src/main.rs"));
        assert!(!is_binary_build_input("profiles/jit-dogfood/manifest.toml"));
        assert!(is_binary_build_input("scripts/hooks/pre-commit"));
        assert!(!is_binary_build_input("docs/new-reference.md"));
        assert!(!is_binary_build_input("scripts/new-tool.sh"));
        assert!(binary_build_input_changes(
            std::iter::empty::<&str>(),
            ["docs/new-reference.md", "scripts/new-tool.sh"],
        )
        .working_tree
        .is_empty());
    }

    /// REQ-02: changing a packaged live source leaves an installed clean
    /// binary fresh when the path is evaluated through the production
    /// build-input predicate.
    #[test]
    fn test_assess_binary_provenance_fresh_for_profile_package_change() {
        let real = build_info::version_info();
        let build_input_changes = binary_build_input_changes(
            ["profiles/jit-dogfood/manifest.toml"],
            std::iter::empty::<&str>(),
        );
        assert_eq!(
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                Some(real.git_commit),
                true,
                &build_input_changes,
            ),
            BinaryProvenance::Fresh
        );
    }

    /// REQ-03: changing a crate source still makes an installed clean binary
    /// stale when the path is evaluated through the production build-input
    /// predicate.
    #[test]
    fn test_assess_binary_provenance_stale_for_crate_source_change() {
        let real = build_info::version_info();
        let build_input_changes =
            binary_build_input_changes(["crates/jit/src/main.rs"], std::iter::empty::<&str>());
        assert_eq!(
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                Some(real.git_commit),
                true,
                &build_input_changes,
            ),
            BinaryProvenance::Stale(StaleBinaryReason::CommitMismatch {
                built_from: real.git_commit.to_string(),
                head: real.git_commit.to_string(),
            })
        );
    }

    /// REQ-12: the covering rule and git's pathspec matching read one
    /// inventory, and this holds them to selecting the same files.
    ///
    /// The installer hands the inventory's lines to `git status` as pathspecs
    /// while the guard matches them through [`is_binary_build_input`]. Nothing
    /// in either mechanism forces those to agree, so it is asserted here over
    /// this repository's own file set: a root spelled in a way git reads
    /// differently, or an exclusion added to the declaration that a pathspec
    /// cannot express, fails this test rather than silently giving the
    /// installer and the guard different answers.
    #[test]
    fn test_binary_build_inputs_select_the_same_files_as_the_installer_pathspecs() {
        use std::collections::BTreeSet;
        use std::process::Command;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let list = |pathspecs: &[&str]| -> BTreeSet<String> {
            let output = Command::new("git")
                .arg("ls-files")
                .arg("-z")
                .arg("--")
                .args(pathspecs)
                .current_dir(&root)
                .output()
                .expect("git ls-files should run");
            assert!(output.status.success(), "git ls-files should succeed");
            output
                .stdout
                .split(|byte| *byte == 0)
                .filter(|entry| !entry.is_empty())
                .map(|entry| String::from_utf8_lossy(entry).into_owned())
                .collect()
        };

        let by_pathspec = list(&declared_build_input_roots());
        let by_covering_rule: BTreeSet<String> = list(&[])
            .into_iter()
            .filter(|path| is_binary_build_input(path))
            .collect();

        assert!(
            !by_pathspec.is_empty(),
            "the inventory must select some of this repository's files"
        );
        assert_eq!(
            by_pathspec, by_covering_rule,
            "the installer's pathspecs and the guard's covering rule must select the same build inputs"
        );
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
                &BuildInputChanges::default(),
            ),
            BinaryProvenance::Fresh
        );
    }

    #[test]
    fn test_assess_binary_provenance_stale_for_same_head_build_input_change() {
        let real = build_info::version_info();
        let build_input_changes =
            binary_build_input_changes(std::iter::empty::<&str>(), ["crates/jit/src/main.rs"]);
        assert_eq!(
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                Some(real.git_commit),
                true,
                &build_input_changes,
            ),
            BinaryProvenance::Stale(StaleBinaryReason::UncommittedBuildInputs {
                built_from: real.git_commit.to_string(),
                paths: vec!["crates/jit/src/main.rs".to_string()],
            })
        );
    }

    #[test]
    fn test_assess_binary_provenance_prefers_uncommitted_inputs_when_head_also_moved() {
        let real = build_info::version_info();
        let build_input_changes =
            binary_build_input_changes(["crates/jit/src/lib.rs"], ["crates/jit/src/main.rs"]);

        assert_eq!(
            assess_binary_provenance(
                Some(real.git_commit),
                Some(false),
                Some("a-different-known-head"),
                true,
                &build_input_changes,
            ),
            BinaryProvenance::Stale(StaleBinaryReason::UncommittedBuildInputs {
                built_from: real.git_commit.to_string(),
                paths: vec!["crates/jit/src/main.rs".to_string()],
            })
        );
    }
}
