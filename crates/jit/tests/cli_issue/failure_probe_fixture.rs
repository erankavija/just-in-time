//! Shared subprocess fixture for failures recorded in the canonical lever registry.

use super::failure_lever_registry::{failure_lever_registry, FailureLever};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus, Output};
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Observable result of driving a registry invocation through its recorded setup.
pub(crate) struct ForcedFailure {
    pub(crate) path: String,
    pub(crate) argv: Vec<String>,
    pub(crate) expected_failure: String,
    pub(crate) expected_exit: i32,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) status: ExitStatus,
}

/// A registry row remains visible whether it has an invocation or an exemption.
pub(crate) enum RecordedFailure {
    Invoked(ForcedFailure),
    Exempt { path: String, reason: String },
}

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

pub(crate) fn drive_recorded_failure(
    invocation: &super::failure_lever_registry::FailureLeverInvocation,
) -> ForcedFailure {
    let mut fixture = FailureFixture::new();
    invocation.setup.iter().for_each(|step| {
        fixture.apply(
            SetupStep::parse(step)
                .unwrap_or_else(|error| panic!("invalid setup for {}: {error}", invocation.path)),
        );
    });

    let output = fixture.run_jit(&invocation.argv);
    let parser_diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        !(parser_diagnostic.starts_with("error:") && parser_diagnostic.contains("Usage:")),
        "{} failed in argument parsing instead of after dispatch: {parser_diagnostic}",
        invocation.path
    );

    ForcedFailure {
        path: invocation.path.clone(),
        argv: invocation.argv.clone(),
        expected_failure: invocation.expected_failure.clone(),
        expected_exit: invocation.expected_exit,
        stdout: output.stdout,
        stderr: output.stderr,
        status: output.status,
    }
}

struct FailureFixture {
    repository: TempDir,
    environment: BTreeMap<String, Option<String>>,
}

impl FailureFixture {
    fn new() -> Self {
        Self {
            repository: TempDir::new().expect("create failure-probe repository"),
            environment: BTreeMap::new(),
        }
    }

    fn root(&self) -> &Path {
        self.repository.path()
    }

    fn run_jit(&self, args: &[String]) -> Output {
        let mut command = Command::new(jit_binary());
        command.current_dir(self.root()).args(args);
        self.environment
            .iter()
            .for_each(|(name, value)| match value {
                Some(value) => {
                    command.env(name, value);
                }
                None => {
                    command.env_remove(name);
                }
            });
        command.output().expect("run recorded jit invocation")
    }

    fn run_setup_jit(&self, args: &[&str]) -> Output {
        self.run_jit(
            &args
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect::<Vec<_>>(),
        )
    }

    fn require_setup_jit(&self, args: &[&str]) -> Output {
        let output = self.run_setup_jit(args);
        assert!(
            output.status.success(),
            "setup command {args:?} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn run_git(&self, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(self.root())
            .args(args)
            .output()
            .expect("run git setup command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn apply(&mut self, step: SetupStep) {
        match step {
            SetupStep::Init => {
                self.require_setup_jit(&["init"]);
            }
            SetupStep::MalformedIndex => {
                fs::write(self.root().join(".jit/index.json"), "not-json")
                    .expect("write malformed index");
            }
            SetupStep::MalformedConfig => {
                fs::write(self.root().join(".jit/config.toml"), "not = valid = toml")
                    .expect("write malformed config");
            }
            SetupStep::MalformedEventsLog => {
                fs::write(self.root().join(".jit/events.jsonl"), "{\n")
                    .expect("write malformed event log");
            }
            SetupStep::MalformedGateRegistry => {
                fs::write(self.root().join(".jit/gates.toml"), "[[gates]")
                    .expect("write malformed gate registry");
            }
            SetupStep::MalformedProfileRecord => {
                let profiles = self.root().join(".jit/profiles");
                fs::create_dir_all(&profiles).expect("create profile registry directory");
                fs::write(profiles.join("jit-dogfood.json"), "{")
                    .expect("write malformed profile record");
            }
            SetupStep::MalformedServerPid => {
                fs::write(self.root().join(".jit/server.pid.json"), "{")
                    .expect("write malformed server PID file");
            }
            SetupStep::MalformedClaimIndex => {
                fs::write(self.root().join(".git/jit/claims.index.json"), "{")
                    .expect("write malformed claim index");
            }
            SetupStep::MalformedWorktreeIdentity => {
                self.require_setup_jit(&["worktree", "info", "--json"]);
                fs::write(self.root().join(".jit/worktree.json"), "{")
                    .expect("write malformed worktree identity");
            }
            SetupStep::MalformedPreset(name) => {
                let presets = self.root().join(".jit/config/gate-presets");
                fs::create_dir_all(&presets).expect("create preset directory");
                fs::write(presets.join(format!("{name}.json")), "{")
                    .expect("write malformed preset");
            }
            SetupStep::GitRepositoryWithHead => {
                self.run_git(&["init"]);
                self.run_git(&["config", "user.email", "fixture@example.com"]);
                self.run_git(&["config", "user.name", "Failure Fixture"]);
                self.run_git(&["commit", "--allow-empty", "-m", "fixture"]);
            }
            SetupStep::GitRepositoryWithoutHead => {
                self.run_git(&["init"]);
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
            SetupStep::ActiveClaimIndex => {
                let created = self.require_setup_jit(&[
                    "issue",
                    "create",
                    "--title",
                    "Failure fixture claim",
                    "--json",
                ]);
                let created: serde_json::Value = serde_json::from_slice(&created.stdout)
                    .expect("claim fixture issue creation emits JSON");
                let issue_id = created["id"].as_str().expect("created issue id");
                self.require_setup_jit(&[
                    "claim",
                    "acquire",
                    issue_id,
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
            SetupStep::IndexLockDirectory => {
                let lock = self.root().join(".jit/.index.lock");
                if lock.exists() {
                    fs::remove_file(&lock).expect("remove initial index lock file");
                }
                fs::create_dir(&lock).expect("replace index lock with directory");
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
    MalformedGateRegistry,
    MalformedProfileRecord,
    MalformedServerPid,
    MalformedClaimIndex,
    MalformedWorktreeIdentity,
    MalformedPreset(String),
    GitRepositoryWithHead,
    GitRepositoryWithoutHead,
    GitAbsent,
    SetEnvironment(String, String),
    UnsetEnvironment(String),
    ActiveClaimIndex,
    DuplicateGate(String),
    DanglingInvariant(String),
    IndexLockDirectory,
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
            "malformed-gate-registry" => Ok(Self::MalformedGateRegistry),
            "malformed-profile-record" => Ok(Self::MalformedProfileRecord),
            "malformed-server-pid-file" => Ok(Self::MalformedServerPid),
            "malformed-claim-index" => Ok(Self::MalformedClaimIndex),
            "malformed-worktree-identity" => Ok(Self::MalformedWorktreeIdentity),
            "git:repository-with-head" => Ok(Self::GitRepositoryWithHead),
            "git:repository-without-head" => Ok(Self::GitRepositoryWithoutHead),
            "git:absent" => Ok(Self::GitAbsent),
            "claim:index-with-active-lease" => Ok(Self::ActiveClaimIndex),
            "index-lock-directory" => Ok(Self::IndexLockDirectory),
            "no-ready-issue" => Ok(Self::MissingReadyIssue),
            "worktree:primary" => Ok(Self::PrimaryWorktree),
            _ => prefixed("env-unset:")
                .map(Self::UnsetEnvironment)
                .or_else(|| {
                    prefixed("env:").and_then(|assignment| {
                        assignment
                            .split_once('=')
                            .map(|(name, value)| Self::SetEnvironment(name.into(), value.into()))
                    })
                })
                .or_else(|| prefixed("malformed-preset:").map(Self::MalformedPreset))
                .or_else(|| prefixed("duplicate-gate:").map(Self::DuplicateGate))
                .or_else(|| {
                    prefixed("dangling-invariant-enforcement:").map(Self::DanglingInvariant)
                })
                .or_else(|| prefixed("file-absent:").map(Self::MissingFile))
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
            | Self::MalformedGateRegistry
            | Self::MalformedProfileRecord
            | Self::MalformedServerPid
            | Self::MalformedClaimIndex
            | Self::MalformedWorktreeIdentity
            | Self::MalformedPreset(_) => SetupClass::CorruptRepositoryFile,
            Self::GitRepositoryWithHead | Self::GitRepositoryWithoutHead | Self::GitAbsent => {
                SetupClass::GitTopology
            }
            Self::SetEnvironment(_, _) | Self::UnsetEnvironment(_) => SetupClass::Environment,
            Self::ActiveClaimIndex
            | Self::DuplicateGate(_)
            | Self::DanglingInvariant(_)
            | Self::BuiltinPreset(_) => SetupClass::SeededDomainState,
            Self::IndexLockDirectory | Self::MissingFile(_) => SetupClass::FilesystemState,
            Self::PrimaryWorktree => SetupClass::WorktreeTopology,
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
fn test_failure_probe_fixture_reports_nonzero_status_for_each_setup_class() {
    let registry = failure_lever_registry();
    let representatives = registry
        .arms
        .iter()
        .filter_map(|arm| match arm {
            FailureLever::Invocation(invocation) if invocation.expected_exit != 0 => {
                Some(invocation)
            }
            FailureLever::Invocation(_) | FailureLever::Exemption(_) => None,
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
        assert_eq!(result.expected_exit, invocation.expected_exit);
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
