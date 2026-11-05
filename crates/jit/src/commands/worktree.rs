//! Worktree inspection and execution.

use super::CommandExecutor;
use crate::domain::identity::WorktreeId;
use crate::domain::store_divergence::{compare_stores, StoreDivergence, StoreRecords};
use crate::errors::{GitRequirementGap, RequiresGitError};
use crate::repository_state::RECORD_SCRATCH_DIR;
use crate::storage::claim_coordinator::{read_active_leases, Lease};
use crate::storage::git::BaseComparison;
use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{read_exact_store, IssueStore, StorageWarning, RECORD_FILE_NAME};
use anyhow::{anyhow, ensure, Context, Result};
use schemars::JsonSchema;
use serde::Serialize;
use std::process::{Command, ExitStatus};

/// Worktree information for display
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorktreeInfo {
    /// Stable worktree identifier
    pub worktree_id: WorktreeId,
    /// Current git branch
    pub branch: String,
    /// Absolute path to worktree root
    pub root_path: String,
    /// Absolute resolved data root selected for this invocation
    pub data_root: String,
    /// Whether this is the main worktree
    pub is_main_worktree: bool,
    /// Shared .git directory path
    pub common_dir: String,
}

/// Entry in worktree list
#[derive(Debug, Serialize, PartialEq, JsonSchema)]
pub struct WorktreeListEntry {
    /// Stable worktree identifier
    pub worktree_id: WorktreeId,
    /// Current git branch
    pub branch: String,
    /// Absolute path to worktree root
    pub path: String,
    /// Whether this is the main worktree
    pub is_main: bool,
    /// Number of active claims in this worktree
    pub active_claims: usize,
}

pub struct WorktreeOps<'a, S: IssueStore> {
    exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    pub fn worktrees(&self) -> WorktreeOps<'_, S> {
        WorktreeOps { exec: self }
    }

    /// Refuse shared registry edits when the selected data root's checkout diverges
    /// from `origin/main`; absent git, an unresolved base or unrelated histories allow them.
    pub(crate) fn require_common_history_with_main(&self) -> Result<()> {
        let Some(checkout) = self.require_worktree_paths()?.git() else {
            return Ok(());
        };
        match checkout.compare_with_base("origin/main")? {
            BaseComparison::Compared {
                merge_base,
                base_commit,
            } if merge_base != base_commit => anyhow::bail!(
                "Global operations require common history with main\n\n\
                 Your branch has diverged from origin/main. To proceed:\n  \
                 git fetch origin\n  \
                 git rebase origin/main"
            ),
            _ => Ok(()),
        }
    }
}

impl<S: IssueStore> WorktreeOps<'_, S> {
    /// Run a child under the repository write lock, refusing a leftover redo record.
    pub fn exec(&self, program: &str, args: &[String]) -> Result<ExitStatus> {
        let layout = self.exec.repository_layout()?;
        let _guard = self.exec.storage.acquire_repo_write_lock()?;
        let record = layout
            .data_root()
            .join(RECORD_SCRATCH_DIR)
            .join(RECORD_FILE_NAME);
        ensure!(
            !record.exists(),
            "Refusing to run over a leftover redo record at {}: run `jit recover` first",
            record.display()
        );
        Command::new(program)
            .args(args)
            .status()
            .map_err(|error| anyhow!("Failed to run {program}: {error}"))
    }

    /// Current checkout identity, branch and paths, with relocation warnings.
    pub fn info(&self) -> Result<(WorktreeInfo, Vec<StorageWarning>)> {
        let paths = self.exec.require_worktree_paths()?;
        let branch = paths
            .write_checkout()
            .ok_or_else(|| {
                crate::errors::RequiresGitError::new(crate::errors::GitRequirementGap::NoRepository)
            })?
            .head()?
            .ok_or_else(|| {
                crate::errors::RequiresGitError::new(crate::errors::GitRequirementGap::NoCommits)
            })?
            .branch_label()
            .to_string();

        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

        Ok((
            WorktreeInfo {
                worktree_id: identity.worktree_id,
                branch,
                root_path: paths.worktree_root.to_string_lossy().to_string(),
                data_root: self
                    .exec
                    .repository_layout()?
                    .data_root()
                    .to_string_lossy()
                    .into_owned(),
                is_main_worktree: !paths.is_worktree(),
                common_dir: paths.common_dir.to_string_lossy().to_string(),
            },
            warnings,
        ))
    }

    /// Worktrees and active claim counts, with identity warnings.
    pub fn list(&self) -> Result<(Vec<WorktreeListEntry>, Vec<StorageWarning>)> {
        let paths = self.exec.require_worktree_paths()?;
        execute_worktree_list_at(paths, &crate::storage::clock::SystemClock)
    }

    /// Compare exact checkout stores without aggregation or writes; a checkout
    /// without a primary counterpart reports no divergences.
    pub fn store_divergence(&self) -> Result<StoreDivergenceReport> {
        let paths = self.exec.require_worktree_paths()?;
        let checkout_store = paths.local_jit.to_string_lossy().into_owned();

        let Some(reference_root) = paths.primary_data_root()? else {
            return Ok(StoreDivergenceReport {
                checkout_store,
                reference_store: None,
                divergences: Vec::new(),
            });
        };

        let local_layout =
            crate::storage::discover_repository_layout(&paths.worktree_root, &paths.local_jit)?;
        let primary_worktree = paths
            .primary_worktree_root()?
            .ok_or_else(|| anyhow::anyhow!("primary worktree root is unavailable"))?;
        let reference_layout =
            crate::storage::discover_repository_layout(primary_worktree, &reference_root)?;
        let local = read_exact_store(&local_layout)?;
        let reference = read_exact_store(&reference_layout)?;

        Ok(StoreDivergenceReport {
            checkout_store,
            reference_store: Some(reference_root.to_string_lossy().into_owned()),
            divergences: compare_stores(
                StoreRecords {
                    issues: &local.issues,
                    events: &local.events,
                },
                StoreRecords {
                    issues: &reference.issues,
                    events: &reference.events,
                },
            ),
        })
    }
}

/// Group leases already filtered for activeness by worktree.
fn count_claims_for_worktree(leases: &[Lease], worktree_id: &WorktreeId) -> usize {
    leases
        .iter()
        .filter(|lease| &lease.worktree_id == worktree_id)
        .count()
}

/// List against one captured time so every worktree uses the same lease expiry cutoff.
fn execute_worktree_list_at(
    paths: &WorktreePaths,
    clock: &dyn crate::storage::clock::Clock,
) -> Result<(Vec<WorktreeListEntry>, Vec<StorageWarning>)> {
    let git_entries = paths
        .write_checkout()
        .ok_or_else(|| RequiresGitError::new(GitRequirementGap::NoRepository))?
        .list_worktrees()?;

    let leases = read_active_leases(paths, clock.now())?;

    // Enrich git entries with JIT data, collecting any relocation warnings
    // observed while loading each worktree's identity.
    let mut warnings = Vec::new();
    let entries = git_entries
        .iter()
        .filter_map(|entry| entry.branch().map(|branch| (entry.path(), branch)))
        .map(|(worktree_path, branch)| -> Result<WorktreeListEntry> {
            let entry_paths = paths
                .for_worktree_root(worktree_path.to_path_buf())?
                .filter(|entry_paths| entry_paths.local_jit.exists());

            // Load worktree identity - error if the store exists but the identity
            // can't be read
            let worktree_id = match entry_paths {
                Some(entry_paths) => {
                    let (identity, entry_warnings) =
                        load_or_create_worktree_identity_with_warnings(&entry_paths, branch)
                            .with_context(|| {
                                format!(
                                    "Failed to load worktree identity for {}",
                                    worktree_path.display()
                                )
                            })?;
                    warnings.extend(entry_warnings);
                    identity.worktree_id
                }
                // No store for this checkout yet - use branch-based temporary ID
                None => WorktreeId::try_from(format!("wt:{branch}")).with_context(|| {
                    format!("Invalid worktree id fallback from branch '{}'", branch)
                })?,
            };

            // Count active claims for this worktree
            let active_claims = count_claims_for_worktree(&leases, &worktree_id);

            let is_main = paths.is_primary_worktree_path(worktree_path)?;

            Ok(WorktreeListEntry {
                worktree_id,
                branch: branch.to_owned(),
                path: worktree_path.to_string_lossy().into_owned(),
                is_main,
                active_claims,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((entries, warnings))
}

/// The outcome of comparing one checkout's store against its reference store.
///
/// A checkout with no reference store — the primary checkout, and any checkout
/// outside version control — reports no divergences and no reference, so an
/// empty [`Self::divergences`] means "nothing to report here" in every
/// environment.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StoreDivergenceReport {
    /// The store the inspected checkout owns.
    pub checkout_store: String,
    /// The store it was compared against, absent when the checkout has none.
    pub reference_store: Option<String>,
    /// Every record the two stores disagree about, empty when they agree.
    pub divergences: Vec<StoreDivergence>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use anyhow::Result;

    // Use shared test utilities
    use crate::test_utils::setup_test_repo;

    #[test]
    fn test_worktree_info_structure() -> Result<()> {
        let (_temp, _storage) = setup_test_repo()?;

        // This test validates the function exists and returns the right structure
        // Actual git detection will fail in test environment, so we can't fully test it
        // Real testing would need a proper git repo setup

        // Just verify the types are correct by constructing manually
        let info = WorktreeInfo {
            worktree_id: "wt:12345678".parse().unwrap(),
            branch: "main".to_string(),
            root_path: "/path/to/worktree".to_string(),
            data_root: "/path/to/worktree/.jit".to_string(),
            is_main_worktree: true,
            common_dir: "/path/to/.git".to_string(),
        };

        assert_eq!(info.worktree_id.as_str(), "wt:12345678");
        assert_eq!(info.branch, "main");
        assert!(info.is_main_worktree);

        Ok(())
    }

    #[test]
    fn test_worktree_list_entry_structure() {
        // Verify the structure is correct
        let entry = WorktreeListEntry {
            worktree_id: "wt:12345678".parse().unwrap(),
            branch: "main".to_string(),
            path: "/home/user/project".to_string(),
            is_main: true,
            active_claims: 3,
        };

        assert_eq!(entry.worktree_id.as_str(), "wt:12345678");
        assert_eq!(entry.branch, "main");
        assert_eq!(entry.active_claims, 3);
        assert!(entry.is_main);
    }

    #[test]
    fn test_count_claims_for_worktree_groups_leases_by_worktree() {
        let now = chrono::Utc::now();
        let leases: Vec<_> = ["wt:abc123", "wt:abc123", "wt:def456"]
            .into_iter()
            .enumerate()
            .map(|(seed, worktree)| Lease {
                lease_id: uuid::Uuid::new_v4().to_string().parse().unwrap(),
                issue_id: crate::test_utils::issue_id(seed as u128),
                agent_id: "agent:1".parse().unwrap(),
                worktree_id: worktree.parse().unwrap(),
                branch: None,
                ttl_secs: 0,
                acquired_at: now,
                expires_at: None,
                last_beat: now,
            })
            .collect();

        assert_eq!(
            count_claims_for_worktree(&leases, &"wt:abc123".parse().unwrap()),
            2
        );
        assert_eq!(
            count_claims_for_worktree(&leases, &"wt:def456".parse().unwrap()),
            1
        );
        assert_eq!(
            count_claims_for_worktree(&leases, &"wt:xyz789".parse().unwrap()),
            0
        );
    }

    #[test]
    fn test_execute_worktree_list_at_excludes_inactive_leases() {
        use crate::storage::clock::FixedClock;
        use crate::storage::{ClaimCoordinator, FileLocker};
        use chrono::Utc;
        use std::sync::Arc;
        use std::time::Duration as StdDuration;

        let cli = crate::test_cli::JitCli::with_repository(tempfile::TempDir::new().unwrap());
        let root = cli.root();

        // Minimal real git repo so `git worktree list` reports the main worktree.
        std::fs::write(root.join("README.md"), "# test\n").unwrap();
        cli.init_git().unwrap().commit("init").unwrap();

        // Resolve the actual branch so lease worktree_ids match what the command
        // computes (no local .jit -> worktree_id is `wt:<branch>`).
        let branch = cli
            .git(["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let worktree_id = WorktreeId::try_from(format!("wt:{branch}")).unwrap();

        let paths = crate::test_utils::primary_checkout_paths(root);

        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = ClaimCoordinator::new(
            paths.clone(),
            FileLocker::new(StdDuration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            worktree_id,
            "agent:test".parse().unwrap(),
        )
        .with_clock(clock.clone());
        coordinator.init().unwrap();

        let threshold = crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS;
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(86),
                1,
                Some("test lease"),
                crate::storage::claim_coordinator::ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(87),
                threshold * 2,
                Some("test lease"),
                crate::storage::claim_coordinator::ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        coordinator
            .acquire_claim_synchronized(
                &crate::test_utils::issue_id(88),
                0,
                Some("test lease"),
                crate::storage::claim_coordinator::ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();

        let active_total = |clock: &FixedClock| -> usize {
            let (entries, _warnings) = execute_worktree_list_at(&paths, clock).unwrap();
            entries.iter().map(|e| e.active_claims).sum()
        };

        assert_eq!(active_total(clock.as_ref()), 3);
        clock.set(base + chrono::Duration::seconds(threshold as i64 + 1));
        assert_eq!(active_total(clock.as_ref()), 1);
    }

    mod store_divergence {
        fn executor(
            paths: &super::WorktreePaths,
        ) -> super::CommandExecutor<crate::storage::JsonFileStorage> {
            super::CommandExecutor::new(crate::storage::JsonFileStorage::new(&paths.local_jit))
                .with_worktree_paths(paths.clone())
        }

        use super::*;
        use crate::domain::store_divergence::{DivergenceClass, DivergentRecord};
        use crate::domain::{Event, Issue};
        use crate::test_utils::fixture_issue;
        use std::collections::BTreeMap;
        use std::path::{Path, PathBuf};

        /// Two checkouts of one repository, the second linked to the first.
        ///
        /// Answers the temporary directory holding both (kept alive by the
        /// caller) and the linked checkout's authority. Path arithmetic over the
        /// selected root is what decides primary-vs-linked identity, so this
        /// fixture needs no git process.
        fn linked_checkout_paths() -> (tempfile::TempDir, WorktreePaths) {
            let temp = tempfile::TempDir::new().expect("create a temporary directory");
            let primary = temp.path().join("main");
            let linked = temp.path().join("feature");
            std::fs::create_dir_all(primary.join(".jit")).expect("create the primary store");
            std::fs::create_dir_all(linked.join(".jit")).expect("create the linked store");

            let paths = crate::test_utils::linked_checkout_paths(&primary, &linked);
            assert!(paths.is_worktree(), "the fixture models a linked checkout");
            (temp, paths)
        }

        /// Write `issue` at its canonical path in the selected layout.
        fn seed_issue(layout: &crate::repository_state::RepositoryLayout, issue: &Issue) {
            let path = layout
                .resolve(&crate::repository_state::VirtualPath::issue_record(
                    &issue.id,
                ))
                .expect("resolve the issue record path");
            std::fs::create_dir_all(path.parent().expect("issue record has a parent"))
                .expect("create the issue directory");
            std::fs::write(
                path,
                crate::repository_state::serialize_issue(issue).expect("serialize the issue"),
            )
            .expect("write the issue record");
        }

        /// Write `events` into the log of the store at `data_root`.
        fn seed_events(data_root: &Path, events: &[Event]) {
            let log = events
                .iter()
                .map(|event| {
                    format!(
                        "{}\n",
                        serde_json::to_string(event).expect("serialize the event")
                    )
                })
                .collect::<String>();
            std::fs::write(data_root.join("events.jsonl"), log).expect("write the event log");
        }

        /// An event carrying `id`, associated with `issue_id`.
        fn event(id: &str, issue_id: &crate::domain::identity::IssueId, title: &str) -> Event {
            let issue =
                crate::domain::Issue::draft(issue_id.clone(), title.to_string(), String::new());
            crate::test_utils::stamped_event(
                crate::domain::EventBody::draft_issue_created(&issue),
                id,
                chrono::DateTime::UNIX_EPOCH,
            )
        }

        /// Every file under `root`, keyed by its path relative to `root`.
        fn stored_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
            fn visit(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
                let entries = std::fs::read_dir(current).expect("read a store directory");
                for entry in entries {
                    let path = entry.expect("read a store entry").path();
                    if path.is_dir() {
                        visit(root, &path, files);
                    } else {
                        let relative = path
                            .strip_prefix(root)
                            .expect("entries sit under the store root")
                            .to_path_buf();
                        files.insert(relative, std::fs::read(&path).expect("read a store file"));
                    }
                }
            }

            let mut files = BTreeMap::new();
            visit(root, root, &mut files);
            files
        }

        /// The report's findings as (kind, class, id) triples.
        fn findings(
            report: &StoreDivergenceReport,
        ) -> Vec<(DivergentRecord, DivergenceClass, &str)> {
            report
                .divergences
                .iter()
                .map(|divergence| (divergence.record, divergence.class, divergence.id.as_str()))
                .collect()
        }

        /// Seed the two stores so each finding class is present, answering the
        /// local-only issue, the reference-only issue, and the conflicting event.
        fn seed_divergent_stores(
            paths: &WorktreePaths,
            reference_root: &Path,
        ) -> (Issue, Issue, Event) {
            let local_only = fixture_issue(
                "Local only".to_string(),
                "Held by the linked checkout alone".to_string(),
            );
            let reference_only = fixture_issue(
                "Reference only".to_string(),
                "Held by the primary checkout alone".to_string(),
            );
            let shared = fixture_issue(
                "Shared".to_string(),
                "Held by both stores identically".to_string(),
            );

            let local_layout =
                crate::storage::discover_repository_layout(&paths.worktree_root, &paths.local_jit)
                    .expect("resolve the local repository layout");
            let primary_root = paths
                .primary_worktree_root()
                .expect("resolve the primary worktree")
                .expect("a linked checkout has a primary worktree");
            let reference_layout =
                crate::storage::discover_repository_layout(primary_root, reference_root)
                    .expect("resolve the primary repository layout");
            seed_issue(&local_layout, &local_only);
            seed_issue(&local_layout, &shared);
            seed_issue(&reference_layout, &reference_only);
            seed_issue(&reference_layout, &shared);

            let conflicting = event("shared-event", &shared.id, "The linked checkout's value");
            seed_events(&paths.local_jit, std::slice::from_ref(&conflicting));
            seed_events(
                reference_root,
                &[event(
                    "shared-event",
                    &shared.id,
                    "The primary checkout's value",
                )],
            );

            (local_only, reference_only, conflicting)
        }

        #[test]
        fn test_execute_worktree_store_divergence_surfaces_every_finding_class_between_the_two_stores(
        ) {
            let (_temp, paths) = linked_checkout_paths();
            let reference_root = paths
                .primary_data_root()
                .expect("resolve the primary store")
                .expect("a linked checkout has a primary store");
            let (local_only, reference_only, conflicting) =
                seed_divergent_stores(&paths, &reference_root);

            let report = executor(&paths)
                .worktrees()
                .store_divergence()
                .expect("compare the two checkout stores");

            let reported = findings(&report);
            assert!(
                reported.contains(&(
                    DivergentRecord::Issue,
                    DivergenceClass::LocalOnly,
                    local_only.id.as_str()
                )),
                "the issue only this checkout holds is reported as local-only: {reported:?}"
            );
            assert!(
                reported.contains(&(
                    DivergentRecord::Issue,
                    DivergenceClass::ReferenceOnly,
                    reference_only.id.as_str()
                )),
                "the issue only the primary holds is reported as reference-only: {reported:?}"
            );
            assert!(
                reported.contains(&(
                    DivergentRecord::Event,
                    DivergenceClass::Conflicting,
                    conflicting.id()
                )),
                "the event both logs hold with different values is a conflict: {reported:?}"
            );
            assert_eq!(
                report.reference_store.as_deref(),
                Some(reference_root.to_string_lossy().as_ref()),
                "the report names the primary store it compared against"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_leaves_both_checkout_stores_unchanged() {
            let (_temp, paths) = linked_checkout_paths();
            let reference_root = paths
                .primary_data_root()
                .expect("resolve the primary store")
                .expect("a linked checkout has a primary store");
            seed_divergent_stores(&paths, &reference_root);

            let before = (
                stored_bytes(&paths.local_jit),
                stored_bytes(&reference_root),
            );
            let report = executor(&paths)
                .worktrees()
                .store_divergence()
                .expect("compare the two checkout stores");
            assert!(
                !report.divergences.is_empty(),
                "the read-only property is asserted over a comparison that found something"
            );

            assert_eq!(
                (
                    stored_bytes(&paths.local_jit),
                    stored_bytes(&reference_root)
                ),
                before,
                "the check writes to neither store"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_reports_nothing_for_a_linked_checkout_agreeing_with_the_primary(
        ) {
            let (_temp, paths) = linked_checkout_paths();
            let reference_root = paths
                .primary_data_root()
                .expect("resolve the primary store")
                .expect("a linked checkout has a primary store");

            let issue = fixture_issue(
                "Agreed".to_string(),
                "Both checkouts hold this record".to_string(),
            );
            let events = [event("shared-event", &issue.id, "Both logs hold this")];
            let local_layout =
                crate::storage::discover_repository_layout(&paths.worktree_root, &paths.local_jit)
                    .unwrap();
            let primary_root = paths.primary_worktree_root().unwrap().unwrap();
            let reference_layout =
                crate::storage::discover_repository_layout(primary_root, &reference_root).unwrap();
            for layout in [&local_layout, &reference_layout] {
                seed_issue(layout, &issue);
            }
            for store in [paths.local_jit.as_path(), reference_root.as_path()] {
                seed_events(store, &events);
            }

            let report = executor(&paths)
                .worktrees()
                .store_divergence()
                .expect("compare the two checkout stores");

            assert_eq!(
                findings(&report),
                Vec::new(),
                "a linked checkout whose store agrees with the primary reports no findings"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_reports_nothing_in_the_primary_checkout() {
            let temp = tempfile::TempDir::new().expect("create a temporary directory");
            let primary = temp.path().join("main");
            std::fs::create_dir_all(primary.join(".jit")).expect("create the primary store");

            let paths = crate::test_utils::primary_checkout_paths(&primary);
            assert!(paths.is_main_worktree(), "the fixture models the primary");
            let layout =
                crate::storage::discover_repository_layout(&paths.worktree_root, &paths.local_jit)
                    .unwrap();
            seed_issue(
                &layout,
                &fixture_issue(
                    "Primary record".to_string(),
                    "The primary checkout's own store".to_string(),
                ),
            );

            let report = executor(&paths)
                .worktrees()
                .store_divergence()
                .expect("inspect the primary checkout");

            assert_eq!(
                findings(&report),
                Vec::new(),
                "the primary checkout has no other store to diverge from"
            );
            assert_eq!(
                report.reference_store, None,
                "the primary checkout names no reference store"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_reports_nothing_outside_version_control() {
            let temp = tempfile::TempDir::new().expect("create a temporary directory");
            let data_root = temp.path().join(".jit");
            std::fs::create_dir_all(&data_root).expect("create the store");
            let layout =
                crate::storage::discover_repository_layout(temp.path(), &data_root).unwrap();
            seed_issue(
                &layout,
                &fixture_issue(
                    "Untracked record".to_string(),
                    "Held by a store outside version control".to_string(),
                ),
            );

            let paths = WorktreePaths::detect_for_data_root(&data_root, temp.path())
                .expect("classify a store outside version control");
            assert!(
                !paths.is_git_repository(),
                "the fixture models a store outside version control"
            );

            let report = executor(&paths)
                .worktrees()
                .store_divergence()
                .expect("a store outside version control succeeds rather than erroring");

            assert_eq!(
                findings(&report),
                Vec::new(),
                "a store outside version control has no checkout to compare against"
            );
            assert_eq!(
                report.reference_store, None,
                "a store outside version control names no reference store"
            );
        }
    }
}
