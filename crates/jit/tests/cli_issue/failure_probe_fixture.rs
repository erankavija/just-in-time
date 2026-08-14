//! Shared subprocess fixture for failures recorded in the canonical lever registry.

use super::failure_lever_registry::{failure_lever_registry, FailureLever, REGISTRY_TOML};
use anyhow::{bail, Context, Result};
use clap::Parser;
use jit::output::ErrorCode;
use jit::storage::{
    publish_fixture_directory_noreplace, publish_fixture_file_noreplace, FileLocker,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tempfile::TempDir;

const CORPUS_SCHEMA: u32 = 1;
const CORPUS_CACHE_DIRECTORY: &str = "jit-recorded-failure-corpus";
const CORPUS_RECEIPT: &str = "JIT_RECORDED_FAILURE_CORPUS_RECEIPT";
const CORPUS_SETUP_MODE: &str = "JIT_RECORDED_FAILURE_CORPUS_SETUP";
const FIXTURE_CONTRACT: &str = include_str!("failure_probe_fixture.rs");
static DIRECT_CORPUS: OnceLock<Result<Arc<RecordedFailureCorpus>, String>> = OnceLock::new();

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Observable result of driving a registry invocation through its recorded setup.
pub(crate) struct ForcedFailure {
    pub(crate) path: String,
    pub(crate) argv: Vec<String>,
    pub(crate) expected_failure: String,
    pub(crate) expected_code: ErrorCode,
    pub(crate) expected_exit: i32,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CorpusManifest {
    schema_version: u32,
    key: String,
    corpus_sha256: String,
    build_invocations: u32,
    invoked_rows: usize,
    exempt_rows: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CorpusReceipt {
    schema_version: u32,
    run_id: String,
    key: String,
    entry: PathBuf,
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

fn observe_failure_lever(lever: &FailureLever) -> RecordedFailureObservation {
    match drive_failure_lever(lever) {
        RecordedFailure::Invoked(failure) => RecordedFailureObservation::Invoked {
            path: failure.path,
            argv: failure.argv,
            repository_root: failure.repository_root,
            stdout: failure.stdout,
            stderr: failure.stderr,
            status: ObservedStatus::from_exit_status(failure.status),
        },
        RecordedFailure::Exempt { path, reason } => {
            RecordedFailureObservation::Exempt { path, reason }
        }
    }
}

fn build_recorded_failure_corpus() -> RecordedFailureCorpus {
    failure_lever_registry()
        .arms
        .iter()
        .map(observe_failure_lever)
        .collect()
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
                && !stdout.is_empty()
                && status.code() == Some(invocation.expected_exit) =>
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
        })
}

/// The one complete raw observation set used by the three aggregate contracts.
pub(crate) fn recorded_failure_corpus() -> Result<Arc<RecordedFailureCorpus>> {
    if std::env::var_os(CORPUS_RECEIPT).is_some() {
        return read_nextest_corpus().map(Arc::new);
    }
    memoized_corpus(&DIRECT_CORPUS, || {
        let key = corpus_key()?;
        let entry = ensure_corpus_entry(&key, build_recorded_failure_corpus)?;
        read_corpus_entry(&entry, &key)
    })
}

fn memoized_corpus(
    cell: &OnceLock<Result<Arc<RecordedFailureCorpus>, String>>,
    build: impl FnOnce() -> Result<RecordedFailureCorpus>,
) -> Result<Arc<RecordedFailureCorpus>> {
    cell.get_or_init(|| build().map(Arc::new).map_err(|error| format!("{error:#}")))
        .as_ref()
        .map(Arc::clone)
        .map_err(|error| anyhow::anyhow!(error.clone()))
}

fn corpus_cache_root() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("resolve failure corpus test executable")?;
    let profile = executable
        .parent()
        .and_then(Path::parent)
        .context("failure corpus executable has no Cargo profile output")?;
    Ok(profile.join(CORPUS_CACHE_DIRECTORY))
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn hash_file(hasher: &mut Sha256, path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect corpus provenance {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "corpus provenance {} is not an ordinary file",
            path.display()
        );
    }
    hash_field(hasher, &metadata.len().to_le_bytes());
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

fn corpus_key() -> Result<String> {
    corpus_key_from_sources(
        REGISTRY_TOML.as_bytes(),
        FIXTURE_CONTRACT.as_bytes(),
        Path::new(jit_binary()),
        &std::env::current_exe().context("resolve corpus setup executable")?,
    )
}

fn corpus_key_from_sources(
    registry: &[u8],
    fixture_contract: &[u8],
    scenario_executable: &Path,
    setup_executable: &Path,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"recorded-failure-corpus/v1");
    hash_field(&mut hasher, std::env::consts::OS.as_bytes());
    hash_field(&mut hasher, registry);
    hash_field(&mut hasher, fixture_contract);
    hash_file(&mut hasher, scenario_executable)?;
    hash_file(&mut hasher, setup_executable)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn ensure_corpus_entry(
    key: &str,
    build: impl FnOnce() -> RecordedFailureCorpus,
) -> Result<PathBuf> {
    let cache_root = corpus_cache_root()?;
    ensure_corpus_entry_at(&cache_root, key, build)
}

fn ensure_corpus_entry_at(
    cache_root: &Path,
    key: &str,
    build: impl FnOnce() -> RecordedFailureCorpus,
) -> Result<PathBuf> {
    fs::create_dir_all(cache_root)?;
    let lock_root = cache_root.join("locks");
    fs::create_dir_all(&lock_root)?;
    let _lock = FileLocker::new(Duration::from_secs(120))
        .lock_exclusive(&lock_root.join(format!("{key}.lock")))?;
    let entry = cache_root.join(key);
    match fs::symlink_metadata(&entry) {
        Ok(_) => {
            read_corpus_entry(&entry, key)?;
            return Ok(entry);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let staging = tempfile::Builder::new()
        .prefix(".recorded-failure-corpus-")
        .tempdir_in(cache_root)?;
    let corpus = build();
    validate_recorded_failure_corpus(&corpus)?;
    let corpus_bytes = serde_json::to_vec(&corpus)?;
    let corpus_sha256 = format!("{:x}", Sha256::digest(&corpus_bytes));
    let corpus_path = staging.path().join("corpus.json");
    fs::write(&corpus_path, &corpus_bytes)?;
    File::open(&corpus_path)?.sync_all()?;
    let manifest = CorpusManifest {
        schema_version: CORPUS_SCHEMA,
        key: key.to_owned(),
        corpus_sha256,
        build_invocations: 1,
        invoked_rows: corpus
            .iter()
            .filter(|row| matches!(row, RecordedFailureObservation::Invoked { .. }))
            .count(),
        exempt_rows: corpus
            .iter()
            .filter(|row| matches!(row, RecordedFailureObservation::Exempt { .. }))
            .count(),
    };
    let manifest_path = staging.path().join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
    File::open(&manifest_path)?.sync_all()?;
    let staging = staging.keep();
    if let Err(error) = publish_fixture_directory_noreplace(&staging, &entry) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error).context("publish recorded failure corpus without replacement");
    }
    read_corpus_entry(&entry, key)?;
    Ok(entry)
}

fn read_ordinary_file(path: &Path, description: &str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {description} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("{description} {} is not an ordinary file", path.display());
    }
    fs::read(path).with_context(|| format!("read {description} {}", path.display()))
}

fn read_corpus_entry(entry: &Path, expected_key: &str) -> Result<RecordedFailureCorpus> {
    let metadata = fs::symlink_metadata(entry)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("recorded failure corpus entry is not an ordinary directory");
    }
    let names = fs::read_dir(entry)?
        .map(|entry| Ok(entry?.file_name()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    if names != BTreeSet::from(["corpus.json".into(), "manifest.json".into()]) {
        bail!("recorded failure corpus entry contains unexpected files");
    }
    let manifest: CorpusManifest = serde_json::from_slice(&read_ordinary_file(
        &entry.join("manifest.json"),
        "corpus manifest",
    )?)?;
    if manifest.schema_version != CORPUS_SCHEMA
        || manifest.key != expected_key
        || manifest.build_invocations != 1
    {
        bail!("recorded failure corpus manifest identity mismatch");
    }
    let bytes = read_ordinary_file(&entry.join("corpus.json"), "recorded failure corpus")?;
    if format!("{:x}", Sha256::digest(&bytes)) != manifest.corpus_sha256 {
        bail!("recorded failure corpus content mismatch");
    }
    let corpus = serde_json::from_slice(&bytes)?;
    validate_recorded_failure_corpus(&corpus)?;
    let invoked_roots = corpus
        .iter()
        .filter_map(|row| match row {
            RecordedFailureObservation::Invoked {
                repository_root, ..
            } => Some(repository_root),
            RecordedFailureObservation::Exempt { .. } => None,
        })
        .collect::<BTreeSet<_>>();
    if invoked_roots.len() != manifest.invoked_rows
        || manifest.invoked_rows + manifest.exempt_rows != corpus.len()
    {
        bail!("recorded failure corpus does not preserve unique mutable probe roots");
    }
    Ok(corpus)
}

fn prepare_nextest_corpus() -> Result<()> {
    let run_id = std::env::var("NEXTEST_RUN_ID").context("corpus setup has no nextest run id")?;
    let nextest_env = PathBuf::from(
        std::env::var_os("NEXTEST_ENV").context("corpus setup has no nextest env file")?,
    );
    let key = corpus_key()?;
    let entry = ensure_corpus_entry(&key, build_recorded_failure_corpus)?;
    let receipt = CorpusReceipt {
        schema_version: CORPUS_SCHEMA,
        run_id,
        key,
        entry,
    };
    let receipt_bytes = serde_json::to_vec(&receipt)?;
    let sha = format!("{:x}", Sha256::digest(&receipt_bytes));
    let cache_root = corpus_cache_root()?;
    let receipt_path = cache_root.join(format!("run-{}.json", receipt.run_id));
    let staging = tempfile::NamedTempFile::new_in(&cache_root)?;
    fs::write(staging.path(), receipt_bytes)?;
    staging.as_file().sync_all()?;
    publish_fixture_file_noreplace(staging.path(), &receipt_path)?;
    let mut environment = fs::OpenOptions::new().append(true).open(nextest_env)?;
    writeln!(environment, "{CORPUS_RECEIPT}={}", receipt_path.display())?;
    writeln!(environment, "{CORPUS_RECEIPT}_SHA256={sha}")?;
    environment.sync_all()?;
    Ok(())
}

fn read_nextest_corpus() -> Result<RecordedFailureCorpus> {
    let run_id = std::env::var("NEXTEST_RUN_ID").context("corpus consumer has no run id")?;
    let path = PathBuf::from(
        std::env::var_os(CORPUS_RECEIPT).context("recorded failure corpus receipt is absent")?,
    );
    let bytes = read_ordinary_file(&path, "recorded failure corpus receipt")?;
    let expected_sha = std::env::var(format!("{CORPUS_RECEIPT}_SHA256"))
        .context("recorded failure corpus receipt SHA is absent")?;
    let cache_root = corpus_cache_root()?;
    let receipt = validate_receipt(&bytes, &expected_sha, &run_id, &cache_root)?;
    read_corpus_entry(&receipt.entry, &receipt.key)
}

fn validate_receipt(
    bytes: &[u8],
    expected_sha: &str,
    run_id: &str,
    cache_root: &Path,
) -> Result<CorpusReceipt> {
    if format!("{:x}", Sha256::digest(bytes)) != expected_sha {
        bail!("recorded failure corpus receipt content mismatch");
    }
    let receipt: CorpusReceipt = serde_json::from_slice(bytes)?;
    if receipt.schema_version != CORPUS_SCHEMA
        || receipt.run_id != run_id
        || receipt.entry != cache_root.join(&receipt.key)
    {
        bail!("recorded failure corpus receipt does not belong to this nextest run");
    }
    Ok(receipt)
}

pub(crate) fn drive_recorded_failure(
    invocation: &super::failure_lever_registry::FailureLeverInvocation,
) -> ForcedFailure {
    assert_recorded_invocation_parses(&invocation.path, &invocation.argv);
    let mut fixture = FailureFixture::new();
    invocation.setup.iter().for_each(|step| {
        fixture.apply(
            SetupStep::parse(step)
                .unwrap_or_else(|error| panic!("invalid setup for {}: {error}", invocation.path)),
        );
    });

    let output = fixture.run_jit(&invocation.argv);
    let repository_root = fixture.root().to_path_buf();

    ForcedFailure {
        path: invocation.path.clone(),
        argv: invocation.argv.clone(),
        expected_failure: invocation.expected_failure.clone(),
        expected_code: invocation.expected_code,
        expected_exit: invocation.expected_exit,
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
    repository: TempDir,
    environment: BTreeMap<String, Option<String>>,
}

impl FailureFixture {
    fn new() -> Self {
        let repository = TempDir::new().expect("create failure-probe repository");
        let git_ceiling = repository.path().display().to_string();
        Self {
            repository,
            // Git may discover a repository inherited from the host's `/tmp`.
            // Stop discovery at the temporary repository so `git:absent`
            // observes only the fixture it was asked to inspect.
            environment: BTreeMap::from([(
                "GIT_CEILING_DIRECTORIES".to_owned(),
                Some(git_ceiling),
            )]),
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
            Self::MissingFile(_) => SetupClass::FilesystemState,
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
    if std::env::var_os(CORPUS_SETUP_MODE).is_some() {
        prepare_nextest_corpus().expect("prepare the run-bound recorded failure corpus");
    }
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
    if std::env::var_os(CORPUS_SETUP_MODE).is_none() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = OnceLock::new();
        let builds = AtomicUsize::new(0);
        let build = || {
            builds.fetch_add(1, Ordering::Relaxed);
            Ok(vec![RecordedFailureObservation::Exempt {
                path: "version".to_owned(),
                reason: "source-only".to_owned(),
            }])
        };
        let first = memoized_corpus(&cache, build).expect("build tiny corpus");
        let second = memoized_corpus(&cache, build).expect("reuse tiny corpus");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(builds.load(Ordering::Relaxed), 1);

        let failed_cache = OnceLock::new();
        let failed_builds = AtomicUsize::new(0);
        let fail = || {
            failed_builds.fetch_add(1, Ordering::Relaxed);
            bail!("synthetic corpus failure")
        };
        assert!(memoized_corpus(&failed_cache, fail).is_err());
        assert!(memoized_corpus(&failed_cache, fail).is_err());
        assert_eq!(failed_builds.load(Ordering::Relaxed), 1);

        let key_inputs = TempDir::new().expect("create corpus key inputs");
        let scenario = key_inputs.path().join("jit");
        let setup = key_inputs.path().join("cli_issue");
        fs::write(&scenario, b"scenario-a").expect("write scenario input");
        fs::write(&setup, b"setup-a").expect("write setup input");
        let key = corpus_key_from_sources(b"registry-a", b"fixture-a", &scenario, &setup)
            .expect("key corpus inputs");
        assert_ne!(
            key,
            corpus_key_from_sources(b"registry-b", b"fixture-a", &scenario, &setup)
                .expect("key changed registry")
        );
        assert_ne!(
            key,
            corpus_key_from_sources(b"registry-a", b"fixture-b", &scenario, &setup)
                .expect("key changed fixture")
        );
        fs::write(&scenario, b"scenario-b").expect("change scenario bytes");
        assert_ne!(
            key,
            corpus_key_from_sources(b"registry-a", b"fixture-a", &scenario, &setup)
                .expect("key changed scenario")
        );
        fs::write(&scenario, b"scenario-a").expect("restore scenario bytes");
        fs::write(&setup, b"setup-b").expect("change setup bytes");
        assert_ne!(
            key,
            corpus_key_from_sources(b"registry-a", b"fixture-a", &scenario, &setup)
                .expect("key changed setup")
        );

        let cache_root = key_inputs.path().join("cache");
        let receipt = CorpusReceipt {
            schema_version: CORPUS_SCHEMA,
            run_id: "run-a".to_owned(),
            key: "key".to_owned(),
            entry: cache_root.join("key"),
        };
        let receipt_bytes = serde_json::to_vec(&receipt).expect("serialize receipt");
        let receipt_sha = format!("{:x}", Sha256::digest(&receipt_bytes));
        assert!(validate_receipt(&receipt_bytes, &receipt_sha, "run-a", &cache_root).is_ok());
        assert!(validate_receipt(&receipt_bytes, "wrong-sha", "run-a", &cache_root).is_err());
        assert!(validate_receipt(&receipt_bytes, &receipt_sha, "run-b", &cache_root).is_err());
        let sibling_receipt = CorpusReceipt {
            entry: cache_root.join("sibling"),
            ..receipt
        };
        let sibling_bytes =
            serde_json::to_vec(&sibling_receipt).expect("serialize sibling receipt");
        let sibling_sha = format!("{:x}", Sha256::digest(&sibling_bytes));
        assert!(validate_receipt(&sibling_bytes, &sibling_sha, "run-a", &cache_root).is_err());

        let occupied = key_inputs.path().join("occupied");
        fs::write(&occupied, b"do-not-replace").expect("seed irregular occupied entry");
        assert!(read_corpus_entry(&occupied, "key").is_err());
        assert_eq!(
            fs::read(&occupied).expect("read unchanged occupied entry"),
            b"do-not-replace"
        );

        let synthetic = failure_lever_registry()
            .arms
            .iter()
            .enumerate()
            .map(|(index, lever)| match lever {
                FailureLever::Invocation(invocation) => RecordedFailureObservation::Invoked {
                    path: invocation.path.clone(),
                    argv: invocation.argv.clone(),
                    repository_root: std::env::temp_dir().join(format!("synthetic-{index}")),
                    stdout: b"{}".to_vec(),
                    stderr: Vec::new(),
                    status: ObservedStatus {
                        code: Some(invocation.expected_exit),
                        signal: None,
                    },
                },
                FailureLever::Exemption(exemption) => RecordedFailureObservation::Exempt {
                    path: exemption.path.clone(),
                    reason: exemption.exemption_reason.clone(),
                },
            })
            .collect::<Vec<_>>();
        let corrupt_cache = key_inputs.path().join("corrupt-cache");
        let first_builds = AtomicUsize::new(0);
        let corrupt_entry = ensure_corpus_entry_at(&corrupt_cache, "fixed-key", || {
            first_builds.fetch_add(1, Ordering::Relaxed);
            synthetic.clone()
        })
        .expect("publish a valid occupied entry");
        assert_eq!(first_builds.load(Ordering::Relaxed), 1);
        let reuse_builds = AtomicUsize::new(0);
        ensure_corpus_entry_at(&corrupt_cache, "fixed-key", || {
            reuse_builds.fetch_add(1, Ordering::Relaxed);
            synthetic.clone()
        })
        .expect("reuse a valid occupied entry");
        assert_eq!(reuse_builds.load(Ordering::Relaxed), 0);

        fs::write(corrupt_entry.join("corpus.json"), b"corrupt")
            .expect("corrupt the occupied corpus");
        let corrupt_builds = AtomicUsize::new(0);
        let result = ensure_corpus_entry_at(&corrupt_cache, "fixed-key", || {
            corrupt_builds.fetch_add(1, Ordering::Relaxed);
            synthetic.clone()
        });
        assert!(result.is_err());
        assert_eq!(corrupt_builds.load(Ordering::Relaxed), 0);
        assert_eq!(
            fs::read(corrupt_entry.join("corpus.json")).expect("read corrupt occupant"),
            b"corrupt"
        );
        assert!(
            fs::read_dir(&corrupt_cache)
                .expect("list corrupt cache")
                .all(|entry| !entry
                    .expect("read cache entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".recorded-failure-corpus-")),
            "corruption refusal must not leave staging residue"
        );

        let invoked = failure_lever_registry()
            .arms
            .iter()
            .filter(|arm| matches!(arm, FailureLever::Invocation(_)))
            .count();
        let old_aggregate_work = invoked * 3;
        let new_aggregate_work = invoked;
        assert!(new_aggregate_work * 100 <= old_aggregate_work * 40);

        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace is two levels above the jit crate");
        let helper = workspace.join("scripts/setup-recorded-failure-corpus.sh");
        let self_test = Command::new(&helper)
            .arg("--self-test")
            .current_dir(workspace)
            .output()
            .expect("run recorded-failure setup self-test");
        assert!(
            self_test.status.success()
                && String::from_utf8_lossy(&self_test.stdout).contains("self-test: PASS"),
            "setup self-test failed: stdout={} stderr={}",
            String::from_utf8_lossy(&self_test.stdout),
            String::from_utf8_lossy(&self_test.stderr)
        );
        let nextest_config = fs::read_to_string(workspace.join(".config/nextest.toml"))
            .expect("read nextest configuration");
        let configured: toml::Value = nextest_config.parse().expect("parse nextest configuration");
        let rules = configured["profile"]["default"]["scripts"]
            .as_array()
            .expect("nextest setup rules");
        let failure_rule = rules
            .iter()
            .find(|rule| rule["setup"].as_str() == Some("recorded-failure-corpus"))
            .expect("recorded failure setup rule");
        let filter = failure_rule["filter"]
            .as_str()
            .expect("failure setup filter");
        [
            "failure_envelope_contract_tests::test_recorded_failure_arms_emit_the_canonical_error_envelope",
            "failure_probe_coverage_tests::test_failure_probe_coverage_matches_clap_arms_and_emits_typed_envelopes",
            "payload_stream_purity_tests::test_machine_readable_failures_emit_one_json_document_on_payload_stream",
        ]
        .iter()
        .for_each(|identity| assert!(filter.contains(identity)));

        #[cfg(unix)]
        {
            let signaled = Command::new("sh")
                .args(["-c", "kill -TERM $$"])
                .status()
                .expect("observe a signaled child");
            let observed = ObservedStatus::from_exit_status(signaled);
            assert_eq!(observed.code(), None);
            assert_eq!(observed.signal, Some(15));
            assert!(!observed.success());
        }
    }
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
        assert_eq!(result.expected_code, invocation.expected_code);
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

#[test]
fn test_failure_probe_fixture_keeps_git_absent_from_inheriting_tmp_ancestor() {
    match drive_named_failure("hooks install") {
        RecordedFailure::Invoked(result) => {
            assert_eq!(result.status.code(), Some(1));
            let envelope: serde_json::Value = serde_json::from_slice(&result.stdout)
                .expect("git-absent hook probe emits a JSON error envelope");
            assert_eq!(envelope["error"]["code"], "HOOKS_INSTALL_ERROR");
        }
        RecordedFailure::Exempt { .. } => panic!("hooks install has a recorded invocation"),
    }
}
