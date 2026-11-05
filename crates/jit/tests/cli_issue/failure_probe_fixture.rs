//! Shared subprocess fixture for failures recorded in the canonical lever registry.

use super::failure_lever_registry::{failure_lever_registry, FailureLever, REGISTRY_TOML};
use anyhow::{bail, Context, Result};
use clap::Parser;
use jit::domain::identity::IssueId;
use jit::output::ErrorCode;
use jit::storage::worktree_paths::WorktreePaths;
use jit::test_cli::{created_issue_id, JitCli};
use jit::test_lease_store::{corrupt_lease, LeaseCorruption};
use jit::test_utils::{copy_tree_exact, keyed_fixture, keyed_fixture_key, CRATE_SOURCE_SHA256};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const CORPUS_FAMILY: &str = "recorded-failure-corpus";
const CORPUS_FILE: &str = "corpus.json";
/// How a recorded failure is driven, which the crate source does not cover.
const FIXTURE_CONTRACT: &str = include_str!("failure_probe_fixture.rs");

/// Observable result of driving a registry invocation through its recorded setup.
pub(crate) struct ForcedFailure {
    pub(crate) path: String,
    pub(crate) argv: Vec<String>,
    pub(crate) expected_failure: String,
    pub(crate) expected_code: ErrorCode,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) status: ExitStatus,
    pub(crate) repository_root: PathBuf,
}

/// A registry row remains visible whether it has an invocation or an exemption.
pub(crate) enum RecordedFailure {
    Invoked(ForcedFailure),
    Exempt { path: String, reason: String },
}

/// Immutable raw process observation shared by the three aggregate contracts.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RecordedFailureObservation {
    Invoked {
        path: String,
        argv: Vec<String>,
        repository_root: PathBuf,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        status: ObservedStatus,
    },
    Exempt {
        path: String,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservedStatus {
    code: Option<i32>,
    signal: Option<i32>,
}

impl ObservedStatus {
    fn from_exit_status(status: ExitStatus) -> Self {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        Self {
            code: status.code(),
            #[cfg(unix)]
            signal: status.signal(),
            #[cfg(not(unix))]
            signal: None,
        }
    }

    pub(crate) fn code(self) -> Option<i32> {
        self.code
    }

    pub(crate) fn success(self) -> bool {
        self.code == Some(0) && self.signal.is_none()
    }
}

impl RecordedFailureObservation {
    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Invoked { path, .. } | Self::Exempt { path, .. } => path,
        }
    }
}

pub(crate) type RecordedFailureCorpus = Vec<RecordedFailureObservation>;

/// Names the registry-defined arm to drive. Where one arm has several measured
/// invocations, the first committed row is the canonical representative.
pub(crate) fn drive_named_failure(path: &str) -> RecordedFailure {
    let registry = failure_lever_registry();
    let lever = registry
        .arms
        .iter()
        .find(|lever| lever.path() == path)
        .unwrap_or_else(|| panic!("failure-lever registry has no command arm `{path}`"));
    drive_failure_lever(lever)
}

/// Returns the fixture surface directly from the canonical registry.
pub(crate) fn recorded_arm_paths() -> BTreeSet<String> {
    failure_lever_registry()
        .arms
        .iter()
        .map(FailureLever::path)
        .map(str::to_owned)
        .collect()
}

/// Drives exactly the invocation or exemption declared by one canonical registry row.
pub(crate) fn drive_failure_lever(lever: &FailureLever) -> RecordedFailure {
    match lever {
        FailureLever::Invocation(invocation) => {
            RecordedFailure::Invoked(drive_recorded_failure(invocation))
        }
        FailureLever::Exemption(exemption) => RecordedFailure::Exempt {
            path: exemption.path.clone(),
            reason: exemption.exemption_reason.clone(),
        },
    }
}

fn observe_failure_lever(lever: &FailureLever, init_baseline: &Path) -> RecordedFailureObservation {
    match lever {
        FailureLever::Invocation(invocation) => {
            let failure = drive_recorded_failure_from(invocation, Some(init_baseline));
            RecordedFailureObservation::Invoked {
                path: failure.path,
                argv: failure.argv,
                repository_root: failure.repository_root,
                stdout: failure.stdout,
                stderr: failure.stderr,
                status: ObservedStatus::from_exit_status(failure.status),
            }
        }
        FailureLever::Exemption(exemption) => RecordedFailureObservation::Exempt {
            path: exemption.path.clone(),
            reason: exemption.exemption_reason.clone(),
        },
    }
}

fn build_recorded_failure_corpus() -> Result<RecordedFailureCorpus> {
    let init_baseline = JitCli::initialised().context("initialise the corpus baseline")?;
    let arms = failure_lever_registry().arms;
    let next = AtomicUsize::new(0);
    let observe_rows = || {
        std::iter::from_fn(|| {
            let index = next.fetch_add(1, Ordering::Relaxed);
            arms.get(index)
                .map(|lever| (index, observe_failure_lever(lever, init_baseline.root())))
        })
        .collect::<Vec<_>>()
    };
    // Rows share no state, and concurrent syncs share the journal commit they wait on.
    Ok(std::thread::scope(|scope| {
        (0..std::thread::available_parallelism().map_or(1, NonZero::get))
            .map(|_| scope.spawn(observe_rows))
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|worker| worker.join().expect("observe a recorded failure"))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }))
}

fn validate_recorded_failure_corpus(corpus: &RecordedFailureCorpus) -> Result<()> {
    let registry = failure_lever_registry();
    if corpus.len() != registry.arms.len() {
        bail!(
            "recorded failure corpus has {} rows; registry has {}",
            corpus.len(),
            registry.arms.len()
        );
    }
    registry
        .arms
        .iter()
        .zip(corpus)
        .try_for_each(|(lever, observation)| match (lever, observation) {
            (
                FailureLever::Invocation(invocation),
                RecordedFailureObservation::Invoked {
                    path,
                    argv,
                    repository_root,
                    stdout,
                    status,
                    ..
                },
            ) if path == &invocation.path
                && argv == &invocation.argv
                && repository_root.is_absolute()
                && !stdout.is_empty() =>
            {
                Ok(())
            }
            (
                FailureLever::Exemption(exemption),
                RecordedFailureObservation::Exempt { path, reason },
            ) if path == &exemption.path && reason == &exemption.exemption_reason => Ok(()),
            _ => bail!(
                "recorded failure corpus row for `{}` does not match the live registry",
                lever.path()
            ),
        })?;
    let invoked = corpus
        .iter()
        .filter_map(|row| match row {
            RecordedFailureObservation::Invoked {
                repository_root, ..
            } => Some(repository_root),
            RecordedFailureObservation::Exempt { .. } => None,
        })
        .collect::<Vec<_>>();
    if invoked.iter().collect::<BTreeSet<_>>().len() != invoked.len() {
        bail!("recorded failure corpus does not preserve unique mutable probe roots");
    }
    Ok(())
}

/// The one complete raw observation set used by the three aggregate contracts.
pub(crate) fn recorded_failure_corpus() -> Result<RecordedFailureCorpus> {
    let lease = keyed_fixture(CORPUS_FAMILY, &live_corpus_key(), |tree| {
        let corpus = build_recorded_failure_corpus()?;
        validate_recorded_failure_corpus(&corpus)?;
        Ok(fs::write(
            tree.join(CORPUS_FILE),
            serde_json::to_vec(&corpus)?,
        )?)
    })?;
    Ok(serde_json::from_slice(&fs::read(
        lease.tree().join(CORPUS_FILE),
    )?)?)
}

fn live_corpus_key() -> String {
    corpus_key(REGISTRY_TOML, CRATE_SOURCE_SHA256, FIXTURE_CONTRACT)
}

fn corpus_key(registry: &str, crate_source_sha256: &str, fixture_contract: &str) -> String {
    keyed_fixture_key(&[
        registry.as_bytes(),
        crate_source_sha256.as_bytes(),
        fixture_contract.as_bytes(),
    ])
}

pub(crate) fn drive_recorded_failure(
    invocation: &super::failure_lever_registry::FailureLeverInvocation,
) -> ForcedFailure {
    drive_recorded_failure_from(invocation, None)
}

pub(crate) fn drive_named_failure_with_args(path: &str, argv: Vec<String>) -> ForcedFailure {
    let invocation = failure_lever_registry()
        .arms
        .into_iter()
        .find_map(|lever| match lever {
            FailureLever::Invocation(invocation) if invocation.path == path => Some(invocation),
            _ => None,
        })
        .unwrap_or_else(|| panic!("failure-lever registry has no invocation `{path}`"));
    drive_recorded_failure_from_args(&invocation, None, &argv)
}

fn drive_recorded_failure_from(
    invocation: &super::failure_lever_registry::FailureLeverInvocation,
    init_baseline: Option<&Path>,
) -> ForcedFailure {
    drive_recorded_failure_from_args(invocation, init_baseline, &invocation.argv)
}

fn drive_recorded_failure_from_args(
    invocation: &super::failure_lever_registry::FailureLeverInvocation,
    init_baseline: Option<&Path>,
    argv: &[String],
) -> ForcedFailure {
    assert_recorded_invocation_parses(&invocation.path, argv);
    let mut fixture = FailureFixture::new(
        JitCli::uninitialised().expect("create failure-probe repository"),
        init_baseline,
    );
    invocation.setup.iter().for_each(|step| {
        fixture.apply(
            SetupStep::parse(step)
                .unwrap_or_else(|error| panic!("invalid setup for {}: {error}", invocation.path)),
        );
    });

    let output = fixture.run_jit(argv);
    let repository_root = fixture.root().to_path_buf();

    ForcedFailure {
        path: invocation.path.clone(),
        argv: argv.to_vec(),
        expected_failure: invocation.expected_failure.clone(),
        expected_code: invocation.expected_code,
        stdout: output.stdout,
        stderr: output.stderr,
        status: output.status,
        repository_root,
    }
}

fn assert_recorded_invocation_parses(path: &str, argv: &[String]) {
    let parsed = jit::cli::Cli::try_parse_from(
        std::iter::once("jit").chain(argv.iter().map(String::as_str)),
    );
    require_parser_acceptance(path, parsed);
}

fn require_parser_acceptance<T>(path: &str, parsed: Result<T, clap::Error>) {
    if let Err(error) = parsed {
        panic!("recorded invocation for `{path}` is rejected before dispatch: {error}");
    }
}

struct FailureFixture {
    cli: JitCli,
    claimed_issue: Option<IssueId>,
    environment: BTreeMap<String, Option<String>>,
    /// An initialised tree the `init` step copies instead of running `jit init`,
    /// whose syncs across every row would wait on the host's writeback backlog.
    init_baseline: Option<PathBuf>,
}

impl FailureFixture {
    fn new(cli: JitCli, init_baseline: Option<&Path>) -> Self {
        let git_ceiling = cli.root().display().to_string();
        Self {
            cli,
            claimed_issue: None,
            // Git may discover a repository inherited from the host's `/tmp`.
            // Stop discovery at the temporary repository so `git:absent`
            // observes only the fixture it was asked to inspect.
            environment: BTreeMap::from([(
                "GIT_CEILING_DIRECTORIES".to_owned(),
                Some(git_ceiling),
            )]),
            init_baseline: init_baseline.map(Path::to_path_buf),
        }
    }

    fn root(&self) -> &Path {
        self.cli.root()
    }

    fn run_jit<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Output {
        let set = self
            .environment
            .iter()
            .filter_map(|(name, value)| value.as_ref().map(|value| (name, value)));
        let mut command = self.cli.command_with_env(args, set);
        self.environment
            .iter()
            .filter(|(_, value)| value.is_none())
            .for_each(|(name, _)| {
                command.env_remove(name);
            });
        command.output().expect("run recorded jit invocation")
    }

    fn require_setup_jit(&self, args: &[&str]) -> Output {
        let output = self.run_jit(args);
        assert!(
            output.status.success(),
            "setup command {args:?} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn apply(&mut self, step: SetupStep) {
        match step {
            SetupStep::Init => match &self.init_baseline {
                Some(baseline) => {
                    copy_tree_exact(baseline, self.root()).expect("copy the initialised baseline");
                }
                None => {
                    self.require_setup_jit(&["init"]);
                }
            },
            SetupStep::MalformedIndex => {
                fs::write(self.root().join(".jit/index.json"), "not-json")
                    .expect("write malformed index");
            }
            SetupStep::MalformedConfig => {
                fs::write(self.root().join(".jit/config.toml"), "not = valid = toml")
                    .expect("write malformed config");
            }
            SetupStep::PendingRedo => {
                let scratch = self.root().join(".jit/tmp");
                fs::create_dir_all(&scratch).expect("create redo scratch");
                fs::write(scratch.join("redo.record"), b"pending").expect("write pending redo");
            }
            SetupStep::MalformedEventsLog => {
                fs::write(self.root().join(".jit/events.jsonl"), "{\n")
                    .expect("write malformed event log");
            }
            SetupStep::MalformedGateRegistry => {
                fs::write(self.root().join(".jit/gates.toml"), "[[gates]")
                    .expect("write malformed gate registry");
            }
            SetupStep::MalformedServerPid => {
                fs::write(self.root().join(".jit/server.pid.json"), "{")
                    .expect("write malformed server PID file");
            }
            SetupStep::MalformedLeaseRecord => {
                let paths =
                    WorktreePaths::detect_for_data_root(&self.root().join(".jit"), self.root())
                        .unwrap();
                corrupt_lease(
                    &paths.shared_jit,
                    self.claimed_issue.as_ref().expect("claim fixture issue"),
                    LeaseCorruption::MalformedJson,
                )
                .expect("write malformed lease record");
            }
            SetupStep::MalformedWorktreeIdentity => {
                self.require_setup_jit(&["worktree", "info", "--json"]);
                fs::write(self.root().join(".jit/worktree.json"), "{")
                    .expect("write malformed worktree identity");
            }
            SetupStep::MalformedIssueRecord(id) => {
                let issues = self.root().join(".jit/issues");
                fs::create_dir_all(&issues).expect("create issue directory");
                fs::write(issues.join(format!("{id}.json")), "not-json")
                    .expect("write malformed issue record");
            }
            SetupStep::SelectedLinkedWorktree => {
                // The linked checkout only carries a store if HEAD tracks one.
                self.cli
                    .commit("track the jit store")
                    .expect("commit the jit store");
                self.cli
                    .git(["worktree", "add", "linked"])
                    .expect("add the linked worktree");
                self.environment.insert(
                    "JIT_DATA_DIR".to_owned(),
                    Some(self.root().join("linked/.jit").display().to_string()),
                );
            }
            SetupStep::MalformedPreset(name) => {
                let presets = self.root().join(".jit/config/gate-presets");
                fs::create_dir_all(&presets).expect("create preset directory");
                fs::write(presets.join(format!("{name}.json")), "{")
                    .expect("write malformed preset");
            }
            SetupStep::GitRepositoryWithHead => {
                self.cli
                    .init_git()
                    .expect("initialise git")
                    .commit("fixture")
                    .expect("commit the fixture HEAD");
            }
            SetupStep::GitRepositoryWithoutHead => {
                self.cli.init_git().expect("initialise git");
            }
            SetupStep::GitAbsent => {
                assert!(!self.root().join(".git").exists(), "git must remain absent");
            }
            SetupStep::SetEnvironment(name, value) => {
                self.environment.insert(name, Some(value));
            }
            SetupStep::UnsetEnvironment(name) => {
                self.environment.insert(name, None);
            }
            SetupStep::SeedIssue(seed) => {
                let issue = jit::domain::Issue::draft(
                    jit::test_utils::issue_id(seed),
                    "Failure fixture".into(),
                    String::new(),
                );
                std::fs::write(
                    self.root().join(format!(".jit/issues/{}.json", issue.id)),
                    serde_json::to_vec(&issue).unwrap(),
                )
                .unwrap();
            }
            SetupStep::ActiveLease => {
                let created = self.require_setup_jit(&[
                    "issue",
                    "create",
                    "--title",
                    "Failure fixture claim",
                    "--json",
                ]);
                let created: serde_json::Value = serde_json::from_slice(&created.stdout)
                    .expect("claim fixture issue creation emits JSON");
                let issue_id = created_issue_id(&created).expect("created issue id");
                self.claimed_issue = Some(issue_id.parse().expect("valid created issue id"));
                self.require_setup_jit(&[
                    "claim",
                    "acquire",
                    &issue_id,
                    "--ttl",
                    "600",
                    "--agent-id",
                    "agent:failure-fixture",
                    "--json",
                ]);
            }
            SetupStep::DuplicateGate(name) => {
                self.require_setup_jit(&[
                    "gate",
                    "define",
                    &name,
                    "--title",
                    "Failure fixture gate",
                    "--description",
                    "Gate seeded by the shared failure fixture",
                    "--json",
                ]);
            }
            SetupStep::DanglingInvariant(name) => {
                let registry = format!(
                    "[[invariants]]\nid = \"{name}\"\nstatement = \"Failure fixture invariant.\"\nkind = \"enforced\"\nenforced-by = \"@/rule/missing-survey-rule\"\n"
                );
                fs::write(self.root().join(".jit/invariants.toml"), registry)
                    .expect("write dangling invariant registry");
            }
            SetupStep::MissingFile(path) => {
                assert!(
                    !self.root().join(path).exists(),
                    "fixture file must be absent"
                );
            }
            SetupStep::MissingIssue(issue)
            | SetupStep::UnknownGate(issue)
            | SetupStep::UnknownPreset(issue)
            | SetupStep::MissingProjection(issue)
            | SetupStep::MissingProfile(issue)
            | SetupStep::MissingTemplate(issue)
            | SetupStep::InvalidRegex(issue)
            | SetupStep::BuiltinPreset(issue) => {
                assert!(!issue.is_empty(), "recorded setup value must be named");
            }
            SetupStep::MissingReadyIssue => {
                let issues = self.root().join(".jit/issues");
                assert!(
                    fs::read_dir(issues)
                        .expect("read empty issue registry")
                        .next()
                        .is_none(),
                    "no-ready-issue setup starts with no issues"
                );
            }
            SetupStep::PrimaryWorktree => {
                assert!(
                    !self.root().join(".git").exists(),
                    "isolated primary fixture must not be a linked worktree"
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Ord, PartialOrd, Eq, PartialEq)]
enum SetupClass {
    BaseRepository,
    AbsencePrecondition,
    CorruptRepositoryFile,
    GitTopology,
    Environment,
    SeededDomainState,
    FilesystemState,
    WorktreeTopology,
    InvocationInput,
}

enum SetupStep {
    Init,
    MalformedIndex,
    MalformedConfig,
    MalformedEventsLog,
    PendingRedo,
    MalformedGateRegistry,
    MalformedServerPid,
    MalformedLeaseRecord,
    MalformedWorktreeIdentity,
    MalformedIssueRecord(String),
    MalformedPreset(String),
    SelectedLinkedWorktree,
    GitRepositoryWithHead,
    GitRepositoryWithoutHead,
    GitAbsent,
    SetEnvironment(String, String),
    UnsetEnvironment(String),
    ActiveLease,
    /// Seed the issue whose id the canonical-id fixture derives from this
    /// hexadecimal seed.
    SeedIssue(u128),
    DuplicateGate(String),
    DanglingInvariant(String),
    MissingFile(String),
    MissingIssue(String),
    UnknownGate(String),
    UnknownPreset(String),
    MissingReadyIssue,
    MissingProjection(String),
    MissingProfile(String),
    MissingTemplate(String),
    InvalidRegex(String),
    BuiltinPreset(String),
    PrimaryWorktree,
}

impl SetupStep {
    fn parse(value: &str) -> Result<Self, String> {
        let prefixed = |prefix: &str| value.strip_prefix(prefix).map(str::to_owned);
        match value {
            "init" => Ok(Self::Init),
            "malformed-index" => Ok(Self::MalformedIndex),
            "malformed-config-toml" => Ok(Self::MalformedConfig),
            "malformed-events-log" => Ok(Self::MalformedEventsLog),
            "pending-redo" => Ok(Self::PendingRedo),
            "malformed-gate-registry" => Ok(Self::MalformedGateRegistry),
            "malformed-server-pid-file" => Ok(Self::MalformedServerPid),
            "malformed-lease-record" => Ok(Self::MalformedLeaseRecord),
            "malformed-worktree-identity" => Ok(Self::MalformedWorktreeIdentity),
            "git:repository-with-head" => Ok(Self::GitRepositoryWithHead),
            "git:repository-without-head" => Ok(Self::GitRepositoryWithoutHead),
            "git:absent" => Ok(Self::GitAbsent),
            "claim:active-lease" => Ok(Self::ActiveLease),
            "no-ready-issue" => Ok(Self::MissingReadyIssue),
            "worktree:primary" => Ok(Self::PrimaryWorktree),
            "worktree:linked-selected" => Ok(Self::SelectedLinkedWorktree),
            _ => prefixed("env-unset:")
                .map(Self::UnsetEnvironment)
                .or_else(|| {
                    prefixed("env:").and_then(|assignment| {
                        assignment
                            .split_once('=')
                            .map(|(name, value)| Self::SetEnvironment(name.into(), value.into()))
                    })
                })
                .or_else(|| prefixed("malformed-issue-record:").map(Self::MalformedIssueRecord))
                .or_else(|| prefixed("malformed-preset:").map(Self::MalformedPreset))
                .or_else(|| prefixed("duplicate-gate:").map(Self::DuplicateGate))
                .or_else(|| {
                    prefixed("dangling-invariant-enforcement:").map(Self::DanglingInvariant)
                })
                .or_else(|| prefixed("file-absent:").map(Self::MissingFile))
                .or_else(|| {
                    prefixed("seed-issue:")
                        .and_then(|seed| u128::from_str_radix(&seed, 16).ok())
                        .map(Self::SeedIssue)
                })
                .or_else(|| prefixed("no-issue:").map(Self::MissingIssue))
                .or_else(|| prefixed("unknown-gate:").map(Self::UnknownGate))
                .or_else(|| prefixed("unknown-preset:").map(Self::UnknownPreset))
                .or_else(|| prefixed("no-projection:").map(Self::MissingProjection))
                .or_else(|| prefixed("no-profile:").map(Self::MissingProfile))
                .or_else(|| prefixed("no-template:").map(Self::MissingTemplate))
                .or_else(|| prefixed("regex:").map(Self::InvalidRegex))
                .or_else(|| prefixed("builtin-preset:").map(Self::BuiltinPreset))
                .ok_or_else(|| format!("unsupported recorded setup step `{value}`")),
        }
    }

    fn class(&self) -> SetupClass {
        match self {
            Self::Init => SetupClass::BaseRepository,
            Self::MalformedIndex
            | Self::MalformedConfig
            | Self::MalformedEventsLog
            | Self::PendingRedo
            | Self::MalformedGateRegistry
            | Self::MalformedServerPid
            | Self::MalformedLeaseRecord
            | Self::MalformedWorktreeIdentity
            | Self::MalformedIssueRecord(_)
            | Self::MalformedPreset(_) => SetupClass::CorruptRepositoryFile,
            Self::GitRepositoryWithHead | Self::GitRepositoryWithoutHead | Self::GitAbsent => {
                SetupClass::GitTopology
            }
            Self::SetEnvironment(_, _) | Self::UnsetEnvironment(_) => SetupClass::Environment,
            Self::ActiveLease
            | Self::SeedIssue(_)
            | Self::DuplicateGate(_)
            | Self::DanglingInvariant(_)
            | Self::BuiltinPreset(_) => SetupClass::SeededDomainState,
            Self::MissingFile(_) => SetupClass::FilesystemState,
            Self::PrimaryWorktree | Self::SelectedLinkedWorktree => SetupClass::WorktreeTopology,
            Self::InvalidRegex(_) => SetupClass::InvocationInput,
            Self::MissingIssue(_)
            | Self::UnknownGate(_)
            | Self::UnknownPreset(_)
            | Self::MissingReadyIssue
            | Self::MissingProjection(_)
            | Self::MissingProfile(_)
            | Self::MissingTemplate(_) => SetupClass::AbsencePrecondition,
        }
    }
}

#[test]
fn test_failure_probe_fixture_accepts_every_recorded_setup_step() {
    failure_lever_registry()
        .arms
        .iter()
        .filter_map(|arm| match arm {
            FailureLever::Invocation(invocation) => Some(&invocation.setup),
            FailureLever::Exemption(_) => None,
        })
        .flatten()
        .for_each(|step| {
            SetupStep::parse(step).unwrap_or_else(|error| panic!("{step}: {error}"));
        });
}

#[test]
fn test_recorded_failure_corpus_is_served_from_its_keyed_fixture_entry() {
    let corpus = recorded_failure_corpus().expect("load the recorded failure corpus");
    let lease = keyed_fixture(CORPUS_FAMILY, &live_corpus_key(), |_| {
        bail!("the corpus must already be published under its live key")
    })
    .expect("lease the published corpus");
    let published: RecordedFailureCorpus = serde_json::from_slice(
        &fs::read(lease.tree().join(CORPUS_FILE)).expect("read the published corpus"),
    )
    .expect("parse the published corpus");
    assert_eq!(corpus, published);
}

#[test]
fn test_corpus_key_changes_with_the_registry_the_crate_source_and_the_fixture_contract() {
    let key = corpus_key("registry", "crate-source", "contract");
    assert_ne!(key, corpus_key("registry-b", "crate-source", "contract"));
    assert_ne!(key, corpus_key("registry", "crate-source-b", "contract"));
    assert_ne!(key, corpus_key("registry", "crate-source", "contract-b"));
}

fn corpus_conforming_to_the_registry() -> RecordedFailureCorpus {
    failure_lever_registry()
        .arms
        .iter()
        .enumerate()
        .map(|(index, lever)| match lever {
            FailureLever::Invocation(invocation) => RecordedFailureObservation::Invoked {
                path: invocation.path.clone(),
                argv: invocation.argv.clone(),
                repository_root: std::env::temp_dir().join(format!("probe-{index}")),
                stdout: b"{}".to_vec(),
                stderr: Vec::new(),
                status: ObservedStatus {
                    code: Some(1),
                    signal: None,
                },
            },
            FailureLever::Exemption(exemption) => RecordedFailureObservation::Exempt {
                path: exemption.path.clone(),
                reason: exemption.exemption_reason.clone(),
            },
        })
        .collect()
}

#[test]
fn test_validate_recorded_failure_corpus_accepts_a_corpus_conforming_to_the_registry() {
    validate_recorded_failure_corpus(&corpus_conforming_to_the_registry())
        .expect("a conforming corpus validates");
}

#[test]
fn test_validate_recorded_failure_corpus_refuses_a_missing_row() {
    let mut corpus = corpus_conforming_to_the_registry();
    corpus.pop();
    assert!(validate_recorded_failure_corpus(&corpus).is_err());
}

#[test]
fn test_validate_recorded_failure_corpus_refuses_rows_sharing_a_repository_root() {
    let mut corpus = corpus_conforming_to_the_registry();
    let mut roots = corpus.iter_mut().filter_map(|row| match row {
        RecordedFailureObservation::Invoked {
            repository_root, ..
        } => Some(repository_root),
        RecordedFailureObservation::Exempt { .. } => None,
    });
    let first = roots.next().expect("first invocation").clone();
    *roots.next().expect("second invocation") = first;
    assert!(validate_recorded_failure_corpus(&corpus).is_err());
}

#[test]
fn test_failure_probe_fixture_init_baseline_observes_what_a_run_init_observes() {
    let registry = failure_lever_registry();
    let invocation = registry
        .arms
        .iter()
        .find_map(|lever| match lever {
            FailureLever::Invocation(invocation) if invocation.path == "config get" => {
                Some(invocation)
            }
            _ => None,
        })
        .expect("config get has a recorded invocation");
    let baseline = JitCli::initialised().expect("initialise the baseline");

    let run = drive_recorded_failure(invocation);
    let copied = drive_recorded_failure_from(invocation, Some(baseline.root()));

    assert_eq!(copied.status, run.status);
    assert_eq!(copied.stdout, run.stdout);
    assert_eq!(copied.stderr, run.stderr);
}

#[cfg(unix)]
#[test]
fn test_observed_status_reports_a_signaled_child_as_a_failure_without_an_exit_code() {
    let signaled = std::process::Command::new("sh")
        .args(["-c", "kill -TERM $$"])
        .status()
        .expect("observe a signaled child");
    let observed = ObservedStatus::from_exit_status(signaled);
    assert_eq!(observed.code(), None);
    assert_eq!(observed.signal, Some(15));
    assert!(!observed.success());
}

#[test]
fn test_failure_probe_fixture_parser_accepts_every_recorded_invocation() {
    failure_lever_registry()
        .arms
        .iter()
        .filter_map(|lever| match lever {
            FailureLever::Invocation(invocation) => Some(invocation),
            FailureLever::Exemption(_) => None,
        })
        .for_each(|invocation| {
            assert_recorded_invocation_parses(&invocation.path, &invocation.argv);
        });
}

#[test]
fn test_failure_probe_fixture_rejects_parser_error_without_usage_text() {
    assert!(
        std::panic::catch_unwind(|| {
            let parser_error = clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                "synthetic parser rejection without a usage block",
            );
            require_parser_acceptance::<()>("synthetic arm", Err(parser_error));
        })
        .is_err(),
        "any structural parser error must reject the recorded invocation"
    );
}

#[test]
fn test_failure_probe_fixture_reports_nonzero_status_for_each_setup_class() {
    let registry = failure_lever_registry();
    let representatives = registry
        .arms
        .iter()
        .filter_map(|arm| match arm {
            FailureLever::Invocation(invocation) => Some(invocation),
            FailureLever::Exemption(_) => None,
        })
        .flat_map(|invocation| {
            invocation
                .setup
                .iter()
                .map(move |step| (SetupStep::parse(step).unwrap().class(), invocation))
        })
        .fold(
            BTreeMap::new(),
            |mut representatives, (class, invocation)| {
                representatives.entry(class).or_insert(invocation);
                representatives
            },
        );

    let recorded_classes = registry
        .arms
        .iter()
        .filter_map(|arm| match arm {
            FailureLever::Invocation(invocation) => Some(&invocation.setup),
            FailureLever::Exemption(_) => None,
        })
        .flatten()
        .map(|step| SetupStep::parse(step).unwrap().class())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        representatives.keys().copied().collect::<BTreeSet<_>>(),
        recorded_classes
    );
    representatives.into_iter().for_each(|(class, invocation)| {
        let result = drive_recorded_failure(invocation);
        assert_eq!(result.path, invocation.path);
        assert_eq!(result.argv, invocation.argv);
        assert_eq!(result.expected_failure, invocation.expected_failure);
        assert_eq!(result.expected_code, invocation.expected_code);
        assert!(
            !result.status.success(),
            "representative {} for setup class {class:?} succeeded: stdout={} stderr={}",
            invocation.path,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let _captured_streams = (result.stdout, result.stderr);
    });
}

#[test]
fn test_failure_probe_fixture_carries_invocations_and_exemptions_from_registry() {
    failure_lever_registry()
        .arms
        .iter()
        .filter(|lever| matches!(lever, FailureLever::Exemption(_)))
        .for_each(|lever| match drive_failure_lever(lever) {
            RecordedFailure::Exempt { path, reason } => {
                assert_eq!(path, lever.path());
                assert!(!reason.is_empty());
            }
            RecordedFailure::Invoked(_) => panic!("exemption was widened into an invocation"),
        });

    let invocation = failure_lever_registry()
        .arms
        .into_iter()
        .find(|lever| matches!(lever, FailureLever::Invocation(_)))
        .expect("registry carries an invocation");
    match drive_failure_lever(&invocation) {
        RecordedFailure::Invoked(result) => assert_eq!(result.path, invocation.path()),
        RecordedFailure::Exempt { .. } => panic!("invocation was narrowed into an exemption"),
    }
}

#[test]
fn test_failure_probe_fixture_names_arms_from_registry() {
    let registry_paths = failure_lever_registry()
        .arms
        .iter()
        .map(FailureLever::path)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(recorded_arm_paths(), registry_paths);

    match drive_named_failure("config get") {
        RecordedFailure::Invoked(result) => {
            assert_eq!(result.path, "config get");
            assert!(!result.status.success());
        }
        RecordedFailure::Exempt { .. } => panic!("config get has a recorded invocation"),
    }
    match drive_named_failure("version") {
        RecordedFailure::Exempt { path, reason } => {
            assert_eq!(path, "version");
            assert!(!reason.is_empty());
        }
        RecordedFailure::Invoked(_) => panic!("version is a declared exemption"),
    }
}

#[test]
fn test_failure_probe_fixture_keeps_git_absent_from_inheriting_tmp_ancestor() {
    match drive_named_failure("hooks install") {
        RecordedFailure::Invoked(result) => {
            assert!(!result.status.success());
            let envelope: serde_json::Value = serde_json::from_slice(&result.stdout)
                .expect("git-absent hook probe emits a JSON error envelope");
            assert_eq!(envelope["error"]["code"], "REQUIRES_GIT");
            assert_eq!(result.status.code(), Some(10));
            let message = envelope["error"]["message"].as_str().unwrap();
            assert!(message.contains("git init"));

            let human = drive_named_failure_with_args(
                "hooks install",
                vec!["hooks".to_owned(), "install".to_owned()],
            );
            assert!(!human.status.success());
            assert_eq!(human.status.code(), result.status.code());
            assert_eq!(
                String::from_utf8(human.stderr).unwrap(),
                format!("Error: {message}\n")
            );
        }
        RecordedFailure::Exempt { .. } => panic!("hooks install has a recorded invocation"),
    }
}
