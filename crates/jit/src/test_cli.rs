//! Shared fixture for tests that spawn the `jit` binary.
//!
//! A [`JitCli`] owns a temporary repository and builds every `Command` through
//! one convention: working directory at the repository and no inherited
//! `JIT_*` variable.

use crate::domain::relative_path::RootRelativePath;
use crate::repository_state::{
    repository_relative_path, RepositoryRootClass, VirtualPath, BOOTSTRAP_LOCK_FILE,
    EVENTS_LOCK_FILE, INDEX_LOCK_FILE, RECORD_SCRATCH_DIR, REPO_WRITE_LOCK_FILE,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

/// Prefix of every environment variable the product reads.
const PRODUCT_ENV_PREFIX: &str = "JIT_";

/// Git variables that redirect a command away from the repository under test.
const GIT_REDIRECTS: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// Author identity of repositories made by [`init_git_repository`].
pub const GIT_AUTHOR_NAME: &str = "Test User";
pub const GIT_AUTHOR_EMAIL: &str = "test@example.com";

/// A temporary repository and the `jit` processes spawned over it.
///
/// The temporary directory is removed when the last fixture sharing it drops.
pub struct JitCli {
    _dir: Arc<TempDir>,
    root: PathBuf,
}

impl JitCli {
    /// An empty directory with no `.jit/`.
    pub fn uninitialised() -> Result<Self> {
        Ok(Self::with_repository(
            TempDir::new().context("create temporary repository")?,
        ))
    }

    /// A repository initialised by `jit init`.
    pub fn initialised() -> Result<Self> {
        let cli = Self::uninitialised()?;
        cli.run(["init"])?;
        Ok(cli)
    }

    /// A repository built by shared test support; the fixture removes it on drop.
    pub fn with_repository(dir: TempDir) -> Self {
        let root = dir.path().to_path_buf();
        Self {
            _dir: Arc::new(dir),
            root,
        }
    }

    /// A primary checkout initialised by git and `jit init` with one base
    /// commit, and a linked checkout of it, answered as `(primary, linked)`.
    ///
    /// Machine-local `.jit/` runtime state is git-ignored in both checkouts.
    pub fn linked_checkouts() -> Result<(Self, Self)> {
        let dir = Arc::new(TempDir::new().context("create temporary checkouts")?);
        let checkout = |name: &str| Self {
            _dir: Arc::clone(&dir),
            root: dir.path().join(name),
        };
        let (primary, linked) = (checkout("main"), checkout("feature"));
        std::fs::create_dir(primary.root()).context("create the primary checkout")?;
        primary.init_git()?.run(["init"])?;
        std::fs::write(
            primary.root().join(".gitignore"),
            machine_local_ignores()?.join("\n") + "\n",
        )
        .context("ignore machine-local runtime state")?;
        let linked_root = linked.root().to_str().context("utf-8 checkout path")?;
        primary
            .commit("track with jit")?
            .git(["worktree", "add", "-q", linked_root])?;
        Ok((primary, linked))
    }

    /// A primary checkout with one base commit, a linked checkout of it, and a
    /// store `jit init` made outside both, answered as `(primary, linked, store)`.
    /// Neither checkout holds a `.jit`.
    pub fn linked_checkouts_over_external_store() -> Result<(Self, Self, PathBuf)> {
        let dir = Arc::new(TempDir::new().context("create temporary checkouts")?);
        let checkout = |name: &str| Self {
            _dir: Arc::clone(&dir),
            root: dir.path().join(name),
        };
        let (primary, linked) = (checkout("main"), checkout("feature"));
        let store = dir.path().join("store");
        std::fs::create_dir(primary.root()).context("create the primary checkout")?;
        let linked_root = linked.root().to_str().context("utf-8 checkout path")?;
        primary
            .init_git()?
            .commit("base")?
            .git(["worktree", "add", "-q", linked_root])?;
        let init = primary
            .command_with_env(["init"], [("JIT_DATA_DIR", &store)])
            .output()
            .context("spawn jit init")?;
        ensure!(init.status.success(), "jit init failed: {init:?}");
        Ok((primary, linked, store))
    }

    /// The fixture over `path` beneath this one's temporary directory, where a
    /// test moves or nests a checkout.
    pub fn at(&self, path: impl AsRef<Path>) -> Self {
        Self {
            _dir: Arc::clone(&self._dir),
            root: self._dir.path().join(path),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Initialise git in the repository (see [`init_git_repository`]).
    pub fn init_git(&self) -> Result<&Self> {
        init_git_repository(self.root())?;
        Ok(self)
    }

    /// Stage the whole working tree and commit it, empty or not, under the
    /// identity [`init_git_repository`] configures.
    pub fn commit(&self, message: &str) -> Result<&Self> {
        self.git(["add", "-A"])?;
        self.git(["commit", "-q", "--allow-empty", "-m", message])?;
        Ok(self)
    }

    /// Run `git args` in the repository, isolated from the developer's git
    /// configuration, and return its stdout.
    pub fn git<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        git(self.root(), args)
    }

    /// A `jit` command in the repository with `args` and no `JIT_*` variable.
    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command_with_env(args, std::iter::empty::<(&str, &str)>())
    }

    /// Like [`Self::command`], with `env` set for this process alone.
    pub fn command_with_env<I, S, K, V>(
        &self,
        args: I,
        env: impl IntoIterator<Item = (K, V)>,
    ) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        let mut command = self.program_command(jit_binary(), args);
        command.envs(env);
        command
    }

    /// A command running `program` under the environment [`Self::command`]
    /// gives `jit`.
    pub fn program_command<I, S>(&self, program: impl AsRef<OsStr>, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(program);
        command.current_dir(self.root()).args(args);
        product_vars(std::env::vars_os().map(|(name, _)| name)).for_each(|name| {
            command.env_remove(name);
        });
        command
    }

    /// Run `jit args` and return its stdout; fails with the stderr on a nonzero exit.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = self.command(args).output().context("spawn jit")?;
        ensure!(
            output.status.success(),
            "jit failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Run `jit args --json` and parse stdout.
    pub fn run_json<I, S>(&self, args: I) -> Result<serde_json::Value>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let args: Vec<OsString> = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .chain(std::iter::once("--json".into()))
            .collect();
        let stdout = self.run(args)?;
        serde_json::from_str(&stdout).context("parse jit --json output")
    }

    /// Run `jit issue create args --json` and return the new issue's id.
    pub fn create_issue<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let args = ["issue".into(), "create".into()]
            .into_iter()
            .chain(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        created_issue_id(&self.run_json(args)?)
    }
}

/// The id in the `--json` output of a command that created an issue.
pub fn created_issue_id(output: &serde_json::Value) -> Result<String> {
    output
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("no string `id` in {output}"))
}

/// Seeds two issues sharing an id prefix, returning that prefix and the ids.
pub fn seed_ambiguous_prefix(cli: &JitCli) -> Result<(String, Vec<String>)> {
    const PREFIX: &str = "aaaabbbb";
    const IDS: [&str; 2] = [
        "aaaabbbb-1111-4111-8111-111111111111",
        "aaaabbbb-2222-4222-8222-222222222222",
    ];
    let template_id = cli.create_issue(["--title", "Template"])?;
    let issues_dir = cli.root().join(".jit").join("issues");
    let template: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
        issues_dir.join(format!("{template_id}.json")),
    )?)?;
    for id in IDS {
        let mut issue = template.clone();
        issue["id"] = serde_json::json!(id);
        std::fs::write(
            issues_dir.join(format!("{id}.json")),
            serde_json::to_string_pretty(&issue)?,
        )?;
    }
    Ok((PREFIX.to_string(), IDS.map(str::to_string).to_vec()))
}

/// Initialise git in `path` with a fixed author identity and unsigned commits,
/// ignoring the developer's global and system git configuration.
pub fn init_git_repository(path: &Path) -> Result<()> {
    git(path, ["init", "--initial-branch=main"])?;
    git(path, ["config", "user.name", GIT_AUTHOR_NAME])?;
    git(path, ["config", "user.email", GIT_AUTHOR_EMAIL])?;
    git(path, ["config", "commit.gpgsign", "false"]).map(drop)
}

/// Initialise git in `path` as [`init_git_repository`] does and record an empty
/// commit, so `HEAD` names a branch.
pub fn init_git_repository_with_commit(path: &Path) -> Result<()> {
    init_git_repository(path)?;
    git(path, ["commit", "-q", "--allow-empty", "-m", "init"]).map(drop)
}

fn git<I, S>(path: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    git_stdout(git_command(path, args))
}

/// A `git args` command in `path`, isolated from the developer's git
/// configuration whatever `HOME` it inherits.
fn git_command<I, S>(path: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command
        .current_dir(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    GIT_REDIRECTS.iter().for_each(|name| {
        command.env_remove(name);
    });
    command
}

fn git_stdout(mut command: Command) -> Result<String> {
    let output = command.output().context("spawn git")?;
    if !output.status.success() {
        bail!(
            "git {:?} failed: {}",
            command.get_args().collect::<Vec<_>>(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Repository-relative ignore patterns for the machine-local runtime state a
/// live store writes, named through the storage layout.
fn machine_local_ignores() -> Result<Vec<String>> {
    let data_locks = [
        REPO_WRITE_LOCK_FILE,
        INDEX_LOCK_FILE,
        EVENTS_LOCK_FILE,
        RECORD_SCRATCH_DIR,
    ]
    .into_iter()
    .map(|name| {
        Ok(repository_relative_path(
            RepositoryRootClass::Data,
            &RootRelativePath::parse(name)?,
        ))
    });
    let data_records = [
        VirtualPath::WORKTREE_IDENTITY,
        VirtualPath::SERVER_LOG,
        VirtualPath::SERVER_PID,
    ]
    .into_iter()
    .map(|path| Ok(path.repository_relative()));
    data_locks
        .chain(data_records)
        .chain(std::iter::once(Ok(BOOTSTRAP_LOCK_FILE.to_owned())))
        .collect()
}

/// The product variables among the environment variable `names`.
fn product_vars(names: impl IntoIterator<Item = OsString>) -> impl Iterator<Item = OsString> {
    names
        .into_iter()
        .filter(|name| name.to_string_lossy().starts_with(PRODUCT_ENV_PREFIX))
}

/// The `jit` binary of the running test target: Cargo's per-target
/// `CARGO_BIN_EXE_jit`, else the binary beside the test executable's `deps/`.
pub fn jit_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_jit")
        .map(PathBuf::from)
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            Some(exe.parent()?.parent()?.join("jit"))
        })
        .expect("locate the jit binary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_product_vars_selects_only_product_prefixed_names() {
        let names = ["JIT_AGENT_ID", "PATH", "JIT_DATA_DIR", "NOT_JIT_"].map(OsString::from);

        let selected: Vec<OsString> = product_vars(names).collect();

        assert_eq!(
            selected,
            ["JIT_AGENT_ID", "JIT_DATA_DIR"].map(OsString::from)
        );
    }

    #[test]
    fn test_command_with_env_sets_overrides_for_its_process_alone() {
        let cli = JitCli::uninitialised().unwrap();

        let overridden = cli.command_with_env(["status"], [("JIT_AGENT_ID", "agent:given")]);
        let plain = cli.command(["status"]);
        let value = |command: &Command, name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
        };

        assert_eq!(
            value(&overridden, "JIT_AGENT_ID").as_deref(),
            Some("agent:given")
        );
        assert_eq!(value(&plain, "JIT_AGENT_ID"), None);
    }

    /// A global hook rejects every commit, so a successful commit proves the
    /// fixture's git never reads the configuration under `HOME`.
    #[cfg(unix)]
    #[test]
    fn test_git_command_ignores_global_configuration_under_inherited_home() {
        let home = TempDir::new().unwrap();
        crate::test_utils::write_executable(&home.path().join("pre-commit"), "#!/bin/sh\nexit 1\n")
            .unwrap();
        std::fs::write(
            home.path().join(".gitconfig"),
            format!("[core]\n\thooksPath = {}\n", home.path().display()),
        )
        .unwrap();
        let repo = TempDir::new().unwrap();
        init_git_repository(repo.path()).unwrap();
        let mut commit = git_command(repo.path(), ["commit", "-q", "--allow-empty", "-m", "c"]);
        commit.env("HOME", home.path());

        git_stdout(commit).unwrap();

        assert_eq!(
            git(repo.path(), ["log", "-1", "--format=%s"])
                .unwrap()
                .trim(),
            "c"
        );
    }

    #[test]
    fn test_init_git_repository_commits_without_global_git_configuration() {
        let home = TempDir::new().unwrap();
        std::fs::write(
            home.path().join(".gitconfig"),
            "[user]\n\tname = Developer\n\temail = dev@example.org\n[commit]\n\tgpgsign = true\n",
        )
        .unwrap();
        let repo = TempDir::new().unwrap();

        init_git_repository(repo.path()).unwrap();
        let developer_git = |args: &[&str]| {
            Command::new("git")
                .current_dir(repo.path())
                .env("HOME", home.path())
                .env_remove("GIT_CONFIG_GLOBAL")
                .args(args)
                .output()
                .unwrap()
        };
        let output = developer_git(&["commit", "--allow-empty", "-m", "c"]);
        let author = developer_git(&["log", "-1", "--format=%an <%ae>"]);

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&author.stdout).trim(),
            format!("{GIT_AUTHOR_NAME} <{GIT_AUTHOR_EMAIL}>")
        );
    }

    #[test]
    fn test_created_issue_id_reads_id_field_and_rejects_other_output() {
        let id = crate::test_utils::issue_id(1);
        assert_eq!(
            created_issue_id(&json!({"id": id, "x": 1})).unwrap(),
            id.as_str()
        );
        assert!(created_issue_id(&json!({"count": 0})).is_err());
        assert!(created_issue_id(&json!({"id": 7})).is_err());
    }
}
